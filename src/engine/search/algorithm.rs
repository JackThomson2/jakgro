use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};

use cozy_chess::util::display_uci_move;
use cozy_chess::{
    BitBoard, Board, Color, File, Move, Piece, Rank, Square, get_bishop_moves, get_king_moves,
    get_knight_moves, get_pawn_attacks, get_rook_moves,
};

use super::control::DeadlineWindow;
use super::see::{static_exchange_eval, static_exchange_eval_after};
use super::time::allocate_time;
use super::transposition::{Bound, Entry, TranspositionTable};
use super::{SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore, SearchTelemetry};
use crate::engine::Position;
use crate::engine::evaluation::{
    EvaluationConfig, MATE_SCORE, MATE_THRESHOLD, MAX_PLY, NEG_INFINITY, POS_INFINITY, Score,
    TacticalSnapshot, evaluate_with_config, exchange_outcome, exchange_risk_on, piece_value,
    root_complexity_bonus, style_snapshot, tactical_snapshot,
};
use crate::engine::position::repetition_key;

const DEFAULT_DEPTH: u32 = 4;
const MAX_DEPTH: u32 = 64;
const QUIESCENCE_DEPTH: u32 = 16;
const ASPIRATION_INITIAL: Score = 50;
const VOLATILE_HOLD_ITERATIONS: u8 = 2;
const CONTROL_POLL_INTERVAL_NODES: u64 = 256;
const ITERATION_TIME_MULTIPLIER: u32 = 2;
const ITERATION_TIME_MARGIN: Duration = Duration::from_millis(5);
const STYLED_ROOT_BUDGET_DIVISOR: u64 = 5;
const STYLED_ROOT_TACTICAL_BUDGET_DIVISOR: u64 = 3;
const STYLED_ROOT_MIN_NODES: u64 = 256;
const STYLED_ROOT_MAX_NODES: u64 = 2_048;
const STYLED_ROOT_TACTICAL_MAX_NODES: u64 = 4_096;
const STYLED_ROOT_MAX_VERIFICATIONS: usize = 2;
const ORDINARY_ROOT_MARGIN_MAX: Score = 26;
const WINNING_ROOT_MARGIN_MAX: Score = 20;
const WINNING_ROOT_SCORE: Score = 200;
const LMR_MIN_CHILD_DEPTH: u32 = 3;
const LMR_MIN_MOVE_INDEX: usize = 3;
const LMR_DEEP_CHILD_DEPTH: u32 = 6;
const LMR_DEEP_MOVE_INDEX: usize = 7;
const LMR_VERY_DEEP_CHILD_DEPTH: u32 = 8;
const LMR_VERY_DEEP_MOVE_INDEX: usize = 12;
const NULL_MOVE_MIN_DEPTH: u32 = 4;
const NULL_MOVE_RULE_FIFTY_LIMIT: u8 = 99;
const STATIC_PRUNING_MAX_DEPTH: u32 = 4;
const QUIET_FUTILITY_MAX_DEPTH: u32 = 2;
const STATIC_PRUNING_RULE_FIFTY_LIMIT: u8 = 80;
const REVERSE_FUTILITY_BASE_MARGIN: Score = 100;
const REVERSE_FUTILITY_DEPTH_MARGIN: Score = 140;
const QUIET_FUTILITY_BASE_MARGIN: Score = 120;
const QUIET_FUTILITY_DEPTH_MARGIN: Score = 140;

fn should_poll_control(nodes: u64) -> bool {
    nodes % CONTROL_POLL_INTERVAL_NODES == 0
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IterationDecision {
    Stop,
    Continue,
    Extend,
}

fn next_iteration_decision(
    deadline: Option<DeadlineWindow>,
    iteration_duration: Duration,
    is_volatile: bool,
    extension_used: bool,
) -> IterationDecision {
    let Some(deadline) = deadline else {
        return IterationDecision::Continue;
    };
    let forecast = iteration_duration
        .saturating_mul(ITERATION_TIME_MULTIPLIER)
        .saturating_add(ITERATION_TIME_MARGIN);

    if forecast <= deadline.soft {
        IterationDecision::Continue
    } else if is_volatile && !extension_used && forecast <= deadline.hard {
        IterationDecision::Extend
    } else {
        IterationDecision::Stop
    }
}

#[derive(Debug)]
struct Aborted;
#[derive(Debug, Default)]
struct IterationStability {
    best_move: Option<Move>,
    score: Option<Score>,
    volatile_for: u8,
}

impl IterationStability {
    fn observe(&mut self, best_move: Option<Move>, score: Score) -> bool {
        let best_move_changed = self.best_move.is_some() && self.best_move != best_move;
        let score_changed = self
            .score
            .is_some_and(|previous| previous.abs_diff(score) >= ASPIRATION_INITIAL as u32);
        if best_move_changed || score_changed {
            self.volatile_for = VOLATILE_HOLD_ITERATIONS;
        } else {
            self.volatile_for = self.volatile_for.saturating_sub(1);
        }
        self.best_move = best_move;
        self.score = Some(score);
        self.volatile_for != 0
    }
}

#[derive(Default)]
struct ZobristHasher(u64);

impl Hasher for ZobristHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = hash;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

type RepetitionCounts = HashMap<u64, usize, BuildHasherDefault<ZobristHasher>>;

#[derive(Clone, Debug)]
struct RepetitionTracker {
    keys: Vec<u64>,
    counts: RepetitionCounts,
}

impl RepetitionTracker {
    fn new(history: &[u64]) -> Self {
        let mut counts = RepetitionCounts::default();
        for &key in history {
            *counts.entry(key).or_insert(0) += 1;
        }

        Self {
            keys: history.to_vec(),
            counts,
        }
    }

    fn current_key(&self) -> u64 {
        *self
            .keys
            .last()
            .expect("search repetition history is empty")
    }

    fn push_key(&mut self, key: u64) {
        self.keys.push(key);
        *self.counts.entry(key).or_insert(0) += 1;
    }

    fn pop(&mut self) {
        let key = self.keys.pop().expect("search repetition stack underflow");
        let count = self
            .counts
            .get_mut(&key)
            .expect("search repetition count missing");
        *count -= 1;
        if *count == 0 {
            self.counts.remove(&key);
        }
    }

    fn occurrences(&self, key: u64) -> usize {
        self.counts.get(&key).copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalResult {
    score: Score,
    path_dependent: bool,
}

#[derive(Debug)]
struct NodeResult {
    score: Score,
    path_dependent: bool,
}

#[derive(Debug)]
struct RootSearchResult {
    primary_score: Score,
    selected: NodeResult,
}

impl RootSearchResult {
    fn from_primary(selected: NodeResult) -> Self {
        Self {
            primary_score: selected.score,
            selected,
        }
    }

    fn primary_inside(&self, window: (Score, Score)) -> bool {
        self.primary_score > window.0 && self.primary_score < window.1
    }
}

#[derive(Clone, Debug)]
struct RootMoveEvidence {
    chess_move: Move,
    score: Score,
    bound: Bound,
    child_pv: Vec<Move>,
}

#[derive(Debug)]
struct ConventionalRootResult {
    selected: NodeResult,
    evidence: Vec<RootMoveEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootEvidenceDecision {
    Reject,
    Accept,
    Probe,
}

fn root_evidence_decision(evidence: &RootMoveEvidence, threshold: Score) -> RootEvidenceDecision {
    match evidence.bound {
        Bound::Exact if evidence.score < threshold => RootEvidenceDecision::Reject,
        Bound::Exact => RootEvidenceDecision::Accept,
        Bound::Upper if evidence.score < threshold => RootEvidenceDecision::Reject,
        Bound::Lower if evidence.score >= threshold => RootEvidenceDecision::Accept,
        Bound::Upper | Bound::Lower => RootEvidenceDecision::Probe,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SacrificeState {
    #[default]
    None,
    Accepted,
    Declined,
    Unverified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SacrificeProfile {
    state: SacrificeState,
    settled_exchange: bool,
    offered_cp: Score,
    accepted_cp: Score,
    remaining_offer_cp: Score,
    reply_count: usize,
    attack_gain: Score,
    king_danger_delta: Score,
    legal_checks: Score,
    compensation_signals: u8,
    queens_retained: bool,
    position_stable: bool,
    verified_reply: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RootLineOutcome {
    #[default]
    Live,
    ImmediateDraw,
    RepetitionDraw,
    SearchedDraw,
}

#[derive(Debug)]
struct RootCandidate {
    chess_move: Move,
    score: Score,
    path_dependent: bool,
    interest: i64,
    pv: Vec<Move>,
    sacrifice: SacrificeProfile,
    outcome: RootLineOutcome,
    sterile_simplification: bool,
}

#[derive(Clone, Copy, Debug)]
struct CandidateSeed {
    chess_move: Move,
    interest: i64,
    sacrifice_hint: Score,
}

#[derive(Clone, Debug)]
struct ProbedCandidate {
    seed: CandidateSeed,
    child_pv: Vec<Move>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MoveFacts {
    chess_move: Move,
    attacker: Piece,
    captured: Option<Piece>,
}

impl MoveFacts {
    fn classify(board: &Board, chess_move: Move) -> Self {
        let attacker = board.piece_on(chess_move.from).unwrap_or(Piece::King);
        Self::classify_with_attacker(board, chess_move, attacker)
    }

    fn classify_with_attacker(board: &Board, chess_move: Move, attacker: Piece) -> Self {
        Self {
            chess_move,
            attacker,
            captured: captured_piece(board, chess_move),
        }
    }

    fn see(self, board: &Board, enabled: bool) -> Option<Score> {
        self.captured
            .filter(|&piece| enabled && piece_value(piece) < piece_value(self.attacker))
            .map(|_| static_exchange_eval(board, self.chess_move))
    }

    fn see_after(self, board: &Board, child: &Board, enabled: bool) -> Option<Score> {
        self.captured
            .filter(|&piece| enabled && piece_value(piece) < piece_value(self.attacker))
            .map(|_| static_exchange_eval_after(board, self.chess_move, child))
    }

    fn search_metadata(self, board: &Board, see: Option<Score>) -> MoveMetadata {
        let gives_check = move_gives_check(board, self.chess_move, self.attacker);
        self.metadata(board, gives_check, see)
    }

    fn child_metadata(self, board: &Board, child: &Board, see: Option<Score>) -> MoveMetadata {
        self.metadata(board, !child.checkers().is_empty(), see)
    }

    fn metadata(self, board: &Board, gives_check: bool, see: Option<Score>) -> MoveMetadata {
        let enemy_king = board.king(!board.side_to_move());
        let king_zone_move = (self.chess_move.to.file() as i32 - enemy_king.file() as i32).abs()
            <= 1
            && (self.chess_move.to.rank() as i32 - enemy_king.rank() as i32).abs() <= 1;
        MoveMetadata {
            chess_move: self.chess_move,
            attacker: self.attacker,
            captured: self.captured,
            gives_check,
            attacking_pawn_push: is_attacking_pawn_push(board, self.chess_move),
            castling: self.attacker == Piece::King
                && (self.chess_move.from.file() as i32 - self.chess_move.to.file() as i32).abs()
                    > 1,
            king_zone_move,
            see,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MoveMetadata {
    chess_move: Move,
    attacker: Piece,
    captured: Option<Piece>,
    gives_check: bool,
    attacking_pawn_push: bool,
    castling: bool,
    king_zone_move: bool,
    see: Option<Score>,
}

impl MoveMetadata {
    #[cfg(test)]
    fn classify(board: &Board, chess_move: Move) -> Self {
        Self::classify_for_search(board, chess_move, true)
    }

    #[cfg(test)]
    fn classify_for_search(board: &Board, chess_move: Move, compute_see: bool) -> Self {
        let facts = MoveFacts::classify(board, chess_move);
        facts.search_metadata(board, facts.see(board, compute_see))
    }

    fn classify_with_child(
        board: &Board,
        chess_move: Move,
        child: &Board,
        compute_see: bool,
    ) -> Self {
        let facts = MoveFacts::classify(board, chess_move);
        facts.child_metadata(board, child, facts.see_after(board, child, compute_see))
    }

    fn classify_with_attacker(
        board: &Board,
        chess_move: Move,
        child: &Board,
        compute_see: bool,
        attacker: Piece,
    ) -> Self {
        let facts = MoveFacts::classify_with_attacker(board, chess_move, attacker);
        facts.child_metadata(board, child, facts.see_after(board, child, compute_see))
    }

    fn facts(self) -> MoveFacts {
        MoveFacts {
            chess_move: self.chess_move,
            attacker: self.attacker,
            captured: self.captured,
        }
    }

    fn is_quiet(self) -> bool {
        self.chess_move.promotion.is_none() && self.captured.is_none()
    }

    fn is_tactical(self) -> bool {
        self.chess_move.promotion.is_some() || self.captured.is_some()
    }
}
#[derive(Debug)]
struct PreparedMove {
    metadata: MoveMetadata,
    child: Board,
    order_score: i64,
    root_complexity: Score,
}

#[derive(Debug)]
struct SearchMove {
    metadata: MoveMetadata,
    order_score: i64,
}

#[derive(Clone, Copy, Debug)]
struct PickerMove {
    facts: MoveFacts,
    see: Option<Score>,
    order_score: i64,
}

impl PickerMove {
    fn metadata(self, board: &Board) -> MoveMetadata {
        self.facts.search_metadata(board, self.see)
    }
}

#[derive(Debug, Default)]
struct MovePickerStorage {
    promotions: Vec<PickerMove>,
    good_captures: Vec<PickerMove>,
    quiets: Vec<SearchMove>,
    bad_captures: Vec<PickerMove>,
    failed_quiets: Vec<MoveMetadata>,
    failed_captures: Vec<MoveMetadata>,
}

impl MovePickerStorage {
    fn clear(&mut self) {
        self.promotions.clear();
        self.good_captures.clear();
        self.quiets.clear();
        self.bad_captures.clear();
        self.failed_quiets.clear();
        self.failed_captures.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MovePickerStage {
    Preferred,
    GenerateTacticals,
    Promotions,
    GoodCaptures,
    FirstKiller,
    SecondKiller,
    GenerateQuiets,
    Quiets,
    SortBadCaptures,
    BadCaptures,
    Done,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MovePickerMode {
    Main,
    Quiescence {
        in_check: bool,
        include_quiet_checks: bool,
    },
}

impl MovePickerMode {
    fn needs_quiets(self) -> bool {
        match self {
            Self::Main => true,
            Self::Quiescence {
                in_check,
                include_quiet_checks,
            } => in_check || include_quiet_checks,
        }
    }

    fn accepts_quiet(self, metadata: MoveMetadata) -> bool {
        match self {
            Self::Main => true,
            Self::Quiescence { in_check: true, .. } => true,
            Self::Quiescence {
                include_quiet_checks,
                ..
            } => include_quiet_checks && metadata.gives_check,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MovePickerWork {
    tactical_generations: usize,
    quiet_generations: usize,
    check_detections: usize,
    see_evaluations: usize,
    quiet_sorts: usize,
}

struct MovePicker<'a> {
    board: &'a Board,
    storage: MovePickerStorage,
    preferred: Option<Move>,
    picked_killers: [Option<Move>; 2],
    ply: u32,
    previous: Option<HistoryMove>,
    evaluation: EvaluationConfig,
    mode: MovePickerMode,
    stage: MovePickerStage,
    stage_index: usize,
    emitted: usize,
    #[cfg(test)]
    work: MovePickerWork,
}

impl<'a> MovePicker<'a> {
    fn new(
        board: &'a Board,
        mut storage: MovePickerStorage,
        preferred: Option<Move>,
        ply: u32,
        previous: Option<HistoryMove>,
        evaluation: EvaluationConfig,
        mode: MovePickerMode,
    ) -> Self {
        storage.clear();
        Self {
            board,
            storage,
            preferred: preferred.filter(|chess_move| board.is_legal(*chess_move)),
            picked_killers: [None; 2],
            ply,
            previous,
            evaluation,
            mode,
            stage: MovePickerStage::Preferred,
            stage_index: 0,
            emitted: 0,
            #[cfg(test)]
            work: MovePickerWork::default(),
        }
    }

    fn next(&mut self, ordering: &MoveOrdering) -> Option<(usize, MoveMetadata)> {
        loop {
            match self.stage {
                MovePickerStage::Preferred => {
                    self.stage = MovePickerStage::GenerateTacticals;
                    if let Some(chess_move) = self.preferred {
                        let facts = MoveFacts::classify(self.board, chess_move);
                        return Some(self.emit_candidate(PickerMove {
                            facts,
                            see: None,
                            order_score: 6_000_000,
                        }));
                    }
                }
                MovePickerStage::GenerateTacticals => {
                    self.generate_tacticals(ordering);
                    self.enter(MovePickerStage::Promotions);
                }
                MovePickerStage::Promotions => {
                    if let Some(candidate) = self.storage.promotions.get(self.stage_index).copied()
                    {
                        self.stage_index += 1;
                        return Some(self.emit_candidate(candidate));
                    }
                    self.enter(MovePickerStage::GoodCaptures);
                }
                MovePickerStage::GoodCaptures => {
                    if let Some(candidate) =
                        self.storage.good_captures.get(self.stage_index).copied()
                    {
                        self.stage_index += 1;
                        return Some(self.emit_candidate(candidate));
                    }
                    self.enter(if self.mode == MovePickerMode::Main {
                        MovePickerStage::FirstKiller
                    } else if self.mode.needs_quiets() {
                        MovePickerStage::GenerateQuiets
                    } else {
                        MovePickerStage::SortBadCaptures
                    });
                }
                MovePickerStage::FirstKiller => {
                    self.enter(MovePickerStage::SecondKiller);
                    if let Some(killer) = self.pick_killer(ordering, 0) {
                        return Some(killer);
                    }
                }
                MovePickerStage::SecondKiller => {
                    self.enter(if self.mode.needs_quiets() {
                        MovePickerStage::GenerateQuiets
                    } else {
                        MovePickerStage::SortBadCaptures
                    });
                    if let Some(killer) = self.pick_killer(ordering, 1) {
                        return Some(killer);
                    }
                }
                MovePickerStage::GenerateQuiets => {
                    self.generate_quiets(ordering);
                    self.enter(MovePickerStage::Quiets);
                }
                MovePickerStage::Quiets => {
                    if let Some(candidate) = self.storage.quiets.get(self.stage_index) {
                        let metadata = candidate.metadata;
                        self.stage_index += 1;
                        return Some(self.emit(metadata));
                    }
                    self.enter(MovePickerStage::SortBadCaptures);
                }
                MovePickerStage::SortBadCaptures => {
                    sort_picker_moves(&mut self.storage.bad_captures);
                    self.enter(MovePickerStage::BadCaptures);
                }
                MovePickerStage::BadCaptures => {
                    if let Some(candidate) =
                        self.storage.bad_captures.get(self.stage_index).copied()
                    {
                        self.stage_index += 1;
                        return Some(self.emit_candidate(candidate));
                    }
                    self.enter(MovePickerStage::Done);
                }
                MovePickerStage::Done => return None,
            }
        }
    }

    fn record_failed_quiet(&mut self, metadata: MoveMetadata) {
        if metadata.is_quiet() {
            self.storage.failed_quiets.push(metadata);
        }
    }

    fn failed_quiets(&self) -> &[MoveMetadata] {
        &self.storage.failed_quiets
    }

    fn record_failed_capture(&mut self, metadata: MoveMetadata) {
        if metadata.chess_move.promotion.is_none() && metadata.facts().captured.is_some() {
            self.storage.failed_captures.push(metadata);
        }
    }

    fn failed_captures(&self) -> &[MoveMetadata] {
        &self.storage.failed_captures
    }

    fn into_storage(self) -> MovePickerStorage {
        self.storage
    }

    #[cfg(test)]
    fn work(&self) -> MovePickerWork {
        self.work
    }

    fn enter(&mut self, stage: MovePickerStage) {
        self.stage = stage;
        self.stage_index = 0;
    }

    fn emit(&mut self, metadata: MoveMetadata) -> (usize, MoveMetadata) {
        let index = self.emitted;
        self.emitted += 1;
        (index, metadata)
    }

    fn emit_candidate(&mut self, candidate: PickerMove) -> (usize, MoveMetadata) {
        #[cfg(test)]
        {
            self.work.check_detections += 1;
        }
        self.emit(candidate.metadata(self.board))
    }

    fn pick_killer(
        &mut self,
        ordering: &MoveOrdering,
        slot: usize,
    ) -> Option<(usize, MoveMetadata)> {
        let chess_move = ordering.killers(self.ply)[slot]?;
        if self.preferred == Some(chess_move)
            || self.picked_killers.contains(&Some(chess_move))
            || !self.board.is_legal(chess_move)
        {
            return None;
        }
        let facts = MoveFacts::classify(self.board, chess_move);
        if chess_move.promotion.is_some() || facts.captured.is_some() {
            return None;
        }
        self.picked_killers[slot] = Some(chess_move);
        Some(self.emit_candidate(PickerMove {
            facts,
            see: None,
            order_score: if slot == 0 { 3_000_000 } else { 2_900_000 },
        }))
    }

    fn generate_tacticals(&mut self, ordering: &MoveOrdering) {
        #[cfg(test)]
        {
            self.work.tactical_generations += 1;
        }
        let board = self.board;
        let preferred = self.preferred;
        let evaluation = self.evaluation;
        let promotions = &mut self.storage.promotions;
        let good_captures = &mut self.storage.good_captures;
        let bad_captures = &mut self.storage.bad_captures;
        #[cfg(test)]
        let work = &mut self.work;
        board.generate_moves(|mut piece_moves| {
            piece_moves.to &= tactical_move_targets(board, piece_moves.piece);
            for chess_move in piece_moves {
                if preferred == Some(chess_move) {
                    continue;
                }
                let facts = MoveFacts::classify(board, chess_move);
                if let Some(order_score) = promotion_order_score(chess_move) {
                    promotions.push(PickerMove {
                        facts,
                        see: None,
                        order_score,
                    });
                    continue;
                }
                let compute_see = evaluation.aggression() > 0;
                #[cfg(test)]
                if compute_see
                    && facts
                        .captured
                        .is_some_and(|piece| piece_value(piece) < piece_value(facts.attacker))
                {
                    work.see_evaluations += 1;
                }
                let see = facts.see(board, compute_see);
                let capture_history = ordering.capture_history_score(board.side_to_move(), facts);
                let order_score = capture_order_score(facts, see, evaluation, capture_history)
                    .expect("tactical destination without a capture or promotion");
                let candidate = PickerMove {
                    facts,
                    see,
                    order_score,
                };
                if capture_is_good(facts, see, evaluation) {
                    good_captures.push(candidate);
                } else {
                    bad_captures.push(candidate);
                }
            }
            false
        });
        sort_picker_moves(promotions);
        sort_picker_moves(good_captures);
    }

    fn generate_quiets(&mut self, ordering: &MoveOrdering) {
        #[cfg(test)]
        {
            self.work.quiet_generations += 1;
        }
        let board = self.board;
        let preferred = self.preferred;
        let picked_killers = self.picked_killers;
        let mode = self.mode;
        let quiets = &mut self.storage.quiets;
        #[cfg(test)]
        let work = &mut self.work;
        board.generate_moves(|mut piece_moves| {
            piece_moves.to &= !tactical_move_targets(board, piece_moves.piece);
            for chess_move in piece_moves {
                if preferred == Some(chess_move) || picked_killers.contains(&Some(chess_move)) {
                    continue;
                }
                let facts = MoveFacts::classify(board, chess_move);
                #[cfg(test)]
                {
                    work.check_detections += 1;
                }
                let metadata = facts.search_metadata(board, None);
                if mode.accepts_quiet(metadata) {
                    quiets.push(SearchMove {
                        metadata,
                        order_score: 0,
                    });
                }
            }
            false
        });
        order_search_moves_in_place(
            board,
            quiets,
            None,
            self.ply,
            self.previous,
            ordering,
            self.evaluation,
        );
        #[cfg(test)]
        {
            self.work.quiet_sorts += 1;
        }
    }
}

fn tactical_move_targets(board: &Board, piece: Piece) -> BitBoard {
    let mut targets = board.colors(!board.side_to_move());
    if piece == Piece::Pawn {
        targets |= Rank::First.bitboard() | Rank::Eighth.bitboard();
        if let Some(file) = board.en_passant() {
            targets |= Square::new(file, Rank::Sixth.relative_to(board.side_to_move())).bitboard();
        }
    }
    targets
}

fn capture_is_good(facts: MoveFacts, see: Option<Score>, evaluation: EvaluationConfig) -> bool {
    if evaluation.aggression() > 0
        && let Some(see) = see
    {
        return see >= 0;
    }
    ordering_piece_value(facts.captured.expect("capture facts"))
        >= ordering_piece_value(facts.attacker)
}

fn sort_picker_moves(moves: &mut [PickerMove]) {
    moves.sort_unstable_by(|left, right| {
        right
            .order_score
            .cmp(&left.order_score)
            .then_with(|| move_key(left.facts.chess_move).cmp(&move_key(right.facts.chess_move)))
    });
}

impl PreparedMove {
    fn new(board: &Board, chess_move: Move, compute_see: bool) -> Self {
        let attacker = board.piece_on(chess_move.from).unwrap_or(Piece::King);
        let mut child = board.clone();
        child.play_unchecked_with_piece(chess_move, attacker);
        Self {
            metadata: MoveMetadata::classify_with_attacker(
                board,
                chess_move,
                &child,
                compute_see,
                attacker,
            ),
            child,
            order_score: 0,
            root_complexity: 0,
        }
    }
}

fn late_move_reduction(
    child_depth: u32,
    move_index: usize,
    metadata: MoveMetadata,
    protected: bool,
    in_check: bool,
    pv_node: bool,
    history_score: i32,
) -> u32 {
    if child_depth < LMR_MIN_CHILD_DEPTH
        || move_index < LMR_MIN_MOVE_INDEX
        || !metadata.is_quiet()
        || metadata.gives_check
        || metadata.castling
        || metadata.king_zone_move
        || protected
        || in_check
        || pv_node
    {
        return 0;
    }

    let mut reduction = 1
        + u32::from(child_depth >= LMR_DEEP_CHILD_DEPTH && move_index >= LMR_DEEP_MOVE_INDEX)
        + u32::from(
            child_depth >= LMR_VERY_DEEP_CHILD_DEPTH && move_index >= LMR_VERY_DEEP_MOVE_INDEX,
        );
    if history_score >= LMR_HISTORY_THRESHOLD {
        reduction = reduction.saturating_sub(1);
    } else if history_score <= -LMR_HISTORY_THRESHOLD {
        reduction = reduction.saturating_add(1);
    }
    reduction.min(child_depth.saturating_sub(2))
}

fn reduced_search_needs_research(reduction: u32, score: Score, alpha: Score) -> bool {
    reduction > 0 && score > alpha
}

fn should_prune_quiescence_capture(
    metadata: MoveMetadata,
    in_check: bool,
    recapture_square: Option<cozy_chess::Square>,
    aggression: u8,
    stand_pat: Score,
    alpha: Score,
) -> bool {
    let delta_margin = 50 + Score::from(aggression);
    let material_ceiling = metadata
        .captured
        .map_or(stand_pat, |piece| {
            stand_pat.saturating_add(piece_value(piece))
        })
        .saturating_add(delta_margin);
    aggression > 0
        && !in_check
        && recapture_square.is_some()
        && metadata.captured.is_some()
        && metadata.chess_move.promotion.is_none()
        && !metadata.gives_check
        && !metadata.king_zone_move
        && !metadata.attacking_pawn_push
        && Some(metadata.chess_move.to) != recapture_square
        && metadata
            .see
            .is_some_and(|see| see < -piece_value(Piece::Pawn))
        && material_ceiling <= alpha
}

const HISTORY_MAX: i32 = 16_384;
const CAPTURE_HISTORY_ENTRIES: usize = 2 * 6 * 64 * 6;
const HISTORY_BONUS_SCALE: u32 = 64;
const LMR_HISTORY_THRESHOLD: i32 = HISTORY_MAX / 3;
const CONTINUATION_BUCKETS: usize = 6 * 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistoryMove {
    piece: Piece,
    to: Square,
}

impl HistoryMove {
    fn from_board(board: &Board, chess_move: Move) -> Self {
        Self {
            piece: board.piece_on(chess_move.from).unwrap_or(Piece::King),
            to: chess_move.to,
        }
    }
}

#[derive(Debug)]
struct MoveOrdering {
    killers: Vec<[Option<Move>; 2]>,
    history: Vec<i32>,
    continuation: Vec<i32>,
    capture_history: Vec<i16>,
}

impl MoveOrdering {
    fn new() -> Self {
        Self {
            killers: vec![[None; 2]; MAX_PLY as usize + 1],
            history: vec![0; 2 * 64 * 64],
            continuation: vec![0; CONTINUATION_BUCKETS * CONTINUATION_BUCKETS],
            capture_history: vec![0; CAPTURE_HISTORY_ENTRIES],
        }
    }

    fn killers(&self, ply: u32) -> [Option<Move>; 2] {
        self.killers.get(ply as usize).copied().unwrap_or([None; 2])
    }

    fn history_score(&self, color: Color, chess_move: Move) -> i32 {
        self.history[history_index(color, chess_move)]
    }

    fn continuation_score(&self, previous: HistoryMove, current: HistoryMove) -> i32 {
        self.continuation[continuation_index(previous, current)]
    }

    fn quiet_history_score(
        &self,
        board: &Board,
        chess_move: Move,
        previous: Option<HistoryMove>,
    ) -> i32 {
        let butterfly = self.history_score(board.side_to_move(), chess_move);
        let Some(previous) = previous else {
            return butterfly;
        };
        let continuation =
            self.continuation_score(previous, HistoryMove::from_board(board, chess_move));
        if butterfly == 0 || butterfly.signum() != continuation.signum() {
            return butterfly;
        }
        (butterfly + continuation / 8).clamp(-HISTORY_MAX, HISTORY_MAX)
    }

    fn record_quiet_cutoff(
        &mut self,
        board: &Board,
        previous: Option<HistoryMove>,
        chess_move: Move,
        failed_quiets: &[MoveMetadata],
        ply: u32,
        depth: u32,
    ) {
        self.record_quiet_cutoff_from(
            board,
            previous,
            chess_move,
            failed_quiets.iter().copied(),
            ply,
            depth,
        );
    }

    fn record_quiet_cutoff_from(
        &mut self,
        board: &Board,
        previous: Option<HistoryMove>,
        chess_move: Move,
        failed_quiets: impl Iterator<Item = MoveMetadata>,
        ply: u32,
        depth: u32,
    ) {
        let killers = &mut self.killers[ply.min(MAX_PLY) as usize];
        if killers[0] != Some(chess_move) {
            killers[1] = killers[0];
            killers[0] = Some(chess_move);
        }

        let bonus = history_bonus(depth);
        self.update_history(board.side_to_move(), chess_move, bonus);
        if let Some(previous) = previous {
            self.update_continuation(previous, HistoryMove::from_board(board, chess_move), bonus);
        }
        for failed in failed_quiets.filter(|failed| failed.is_quiet()) {
            self.update_history(board.side_to_move(), failed.chess_move, -bonus);
            if let Some(previous) = previous {
                self.update_continuation(
                    previous,
                    HistoryMove::from_board(board, failed.chess_move),
                    -bonus,
                );
            }
        }
    }

    fn update_history(&mut self, color: Color, chess_move: Move, bonus: i32) {
        update_gravity(&mut self.history[history_index(color, chess_move)], bonus);
    }

    fn update_continuation(&mut self, previous: HistoryMove, current: HistoryMove, bonus: i32) {
        update_gravity(
            &mut self.continuation[continuation_index(previous, current)],
            bonus,
        );
    }

    fn capture_history_score(&self, color: Color, facts: MoveFacts) -> i32 {
        let Some(captured) = facts.captured else {
            return 0;
        };
        i32::from(
            self.capture_history
                [capture_history_index(color, facts.attacker, facts.chess_move.to, captured)],
        )
    }

    fn record_capture_cutoff(
        &mut self,
        board: &Board,
        winner: MoveMetadata,
        failed_captures: &[MoveMetadata],
        depth: u32,
    ) {
        if winner.chess_move.promotion.is_some() || winner.facts().captured.is_none() {
            return;
        }
        let color = board.side_to_move();
        let bonus = history_bonus(depth);
        self.update_capture_history(color, winner.facts(), bonus);
        for failed in failed_captures.iter().copied() {
            if failed.chess_move.promotion.is_none() && failed.facts().captured.is_some() {
                self.update_capture_history(color, failed.facts(), -bonus);
            }
        }
    }

    fn update_capture_history(&mut self, color: Color, facts: MoveFacts, bonus: i32) {
        let Some(captured) = facts.captured else {
            return;
        };
        update_capture_gravity(
            &mut self.capture_history
                [capture_history_index(color, facts.attacker, facts.chess_move.to, captured)],
            bonus,
        );
    }
}

fn history_index(color: Color, chess_move: Move) -> usize {
    ((color as usize * 64 + chess_move.from as usize) * 64) + chess_move.to as usize
}

fn capture_history_index(color: Color, attacker: Piece, to: Square, captured: Piece) -> usize {
    (((color as usize * 6 + attacker as usize) * 64 + to as usize) * 6) + captured as usize
}

fn continuation_index(previous: HistoryMove, current: HistoryMove) -> usize {
    let previous = previous.piece as usize * 64 + previous.to as usize;
    let current = current.piece as usize * 64 + current.to as usize;
    previous * CONTINUATION_BUCKETS + current
}

fn update_gravity(score: &mut i32, bonus: i32) {
    let bounded_bonus = bonus.clamp(-HISTORY_MAX, HISTORY_MAX);
    let gravity = *score * bounded_bonus.abs() / HISTORY_MAX;
    *score = (*score + bounded_bonus - gravity).clamp(-HISTORY_MAX, HISTORY_MAX);
}

fn update_capture_gravity(score: &mut i16, bonus: i32) {
    let bounded_bonus = bonus.clamp(-HISTORY_MAX, HISTORY_MAX);
    let current = i32::from(*score);
    let gravity = current * bounded_bonus.abs() / HISTORY_MAX;
    *score = (current + bounded_bonus - gravity).clamp(-HISTORY_MAX, HISTORY_MAX) as i16;
}

fn history_bonus(depth: u32) -> i32 {
    depth
        .saturating_mul(depth)
        .saturating_mul(HISTORY_BONUS_SCALE)
        .min((HISTORY_MAX / 2) as u32) as i32
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SearchMode {
    #[default]
    Normal,
    NullProbe,
    Verification,
}

impl SearchMode {
    const fn tracks_legal_draws(self) -> bool {
        !matches!(self, Self::NullProbe)
    }

    const fn reads_tt(self) -> bool {
        !matches!(self, Self::NullProbe)
    }

    const fn writes_tt(self) -> bool {
        !matches!(self, Self::NullProbe)
    }

    const fn updates_ordering(self) -> bool {
        matches!(self, Self::Normal)
    }

    const fn allows_null(self) -> bool {
        matches!(self, Self::Normal)
    }
}

const fn null_search_modes() -> (SearchMode, SearchMode) {
    (SearchMode::NullProbe, SearchMode::Verification)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NullMoveBlock {
    Mode,
    Depth,
    PvNode,
    InCheck,
    MateWindow,
    RuleFifty,
    Material,
    StaticEvaluation,
    Unavailable,
}

fn null_move_state_block(
    board: &Board,
    depth: u32,
    beta: Score,
    pv_node: bool,
    mode: SearchMode,
) -> Option<NullMoveBlock> {
    if !mode.allows_null() {
        return Some(NullMoveBlock::Mode);
    }
    if depth < NULL_MOVE_MIN_DEPTH {
        return Some(NullMoveBlock::Depth);
    }
    if pv_node {
        return Some(NullMoveBlock::PvNode);
    }
    if !board.checkers().is_empty() {
        return Some(NullMoveBlock::InCheck);
    }
    if beta.abs() >= MATE_THRESHOLD {
        return Some(NullMoveBlock::MateWindow);
    }
    if board.halfmove_clock() >= NULL_MOVE_RULE_FIFTY_LIMIT {
        return Some(NullMoveBlock::RuleFifty);
    }
    if !null_move_material_ok(board) {
        return Some(NullMoveBlock::Material);
    }
    None
}

fn null_move_static_block(static_evaluation: Score, beta: Score) -> Option<NullMoveBlock> {
    (static_evaluation < beta).then_some(NullMoveBlock::StaticEvaluation)
}

fn make_null_move(board: &Board) -> Result<Board, NullMoveBlock> {
    board.null_move().ok_or(NullMoveBlock::Unavailable)
}

#[cfg(test)]
fn null_move_block(
    board: &Board,
    depth: u32,
    beta: Score,
    pv_node: bool,
    static_evaluation: Score,
    mode: SearchMode,
) -> Option<NullMoveBlock> {
    null_move_state_block(board, depth, beta, pv_node, mode)
        .or_else(|| null_move_static_block(static_evaluation, beta))
        .or_else(|| make_null_move(board).err())
}

fn null_move_material_ok(board: &Board) -> bool {
    let color = board.side_to_move();
    let heavy =
        board.colored_pieces(color, Piece::Rook) | board.colored_pieces(color, Piece::Queen);
    let minors = (board.colored_pieces(color, Piece::Knight)
        | board.colored_pieces(color, Piece::Bishop))
    .len();
    !heavy.is_empty() || minors >= 2
}
fn static_pruning_material_ok(board: &Board) -> bool {
    [Color::White, Color::Black].into_iter().all(|color| {
        let heavy =
            board.colored_pieces(color, Piece::Rook) | board.colored_pieces(color, Piece::Queen);
        let minors = (board.colored_pieces(color, Piece::Knight)
            | board.colored_pieces(color, Piece::Bishop))
        .len();
        !heavy.is_empty() || minors >= 2
    })
}

fn static_pruning_allowed(
    board: &Board,
    depth: u32,
    alpha: Score,
    beta: Score,
    pv_node: bool,
    mode: SearchMode,
) -> bool {
    matches!(mode, SearchMode::Normal)
        && depth <= STATIC_PRUNING_MAX_DEPTH
        && !pv_node
        && board.checkers().is_empty()
        && alpha.abs() < MATE_THRESHOLD
        && beta.abs() < MATE_THRESHOLD
        && board.halfmove_clock() < STATIC_PRUNING_RULE_FIFTY_LIMIT
        && static_pruning_material_ok(board)
}

fn reverse_futility_cutoff(
    static_evaluation: Score,
    beta: Score,
    depth: u32,
    aggression: u8,
) -> bool {
    let margin = REVERSE_FUTILITY_BASE_MARGIN
        + REVERSE_FUTILITY_DEPTH_MARGIN * depth as Score
        + Score::from(aggression);
    static_evaluation.saturating_sub(margin) >= beta
}

#[allow(clippy::too_many_arguments)]
fn should_prune_quiet_move(
    depth: u32,
    move_index: usize,
    metadata: MoveMetadata,
    protected: bool,
    history_score: i32,
    static_evaluation: Score,
    alpha: Score,
    aggression: u8,
) -> bool {
    if depth > QUIET_FUTILITY_MAX_DEPTH
        || move_index == 0
        || protected
        || history_score > 0
        || !metadata.is_quiet()
        || metadata.gives_check
        || metadata.attacking_pawn_push
        || metadata.castling
        || metadata.king_zone_move
    {
        return false;
    }
    let margin = QUIET_FUTILITY_BASE_MARGIN
        + QUIET_FUTILITY_DEPTH_MARGIN * depth as Score
        + Score::from(aggression);
    static_evaluation.saturating_add(margin) <= alpha
}

fn null_move_reduction(depth: u32) -> u32 {
    (2 + depth / 4).min(depth.saturating_sub(1))
}

#[allow(clippy::too_many_arguments)]
fn verified_null_move_cutoff(
    board: &Board,
    history: &mut RepetitionTracker,
    depth: u32,
    ply: u32,
    extensions_used: u8,
    alpha: Score,
    beta: Score,
    static_evaluation: Option<Score>,
    previous_move: Option<HistoryMove>,
    context: &mut SearchContext<'_>,
) -> Result<Option<NodeResult>, Aborted> {
    if !context.null_move_enabled {
        return Ok(None);
    }
    let pv_node = beta.saturating_sub(alpha) > 1;
    if null_move_state_block(board, depth, beta, pv_node, context.mode).is_some() {
        return Ok(None);
    }
    let static_evaluation =
        static_evaluation.unwrap_or_else(|| evaluate_with_config(board, context.scoring));
    if null_move_static_block(static_evaluation, beta).is_some() {
        return Ok(None);
    }
    let Ok(null_board) = make_null_move(board) else {
        return Ok(None);
    };

    context.telemetry.null_move_attempts += 1;
    let (probe_mode, verification_mode) = null_search_modes();
    let reduction = null_move_reduction(depth);
    let null_depth = depth.saturating_sub(reduction).saturating_sub(1);
    let verification_depth = depth.saturating_sub(reduction);
    let original_mode = context.mode;
    context.mode = probe_mode;
    let probe = negamax(
        &null_board,
        history,
        null_depth,
        ply + 1,
        extensions_used,
        -beta,
        -beta + 1,
        None,
        &[],
        context,
    );
    context.mode = original_mode;
    let probe = probe?;
    if -probe.score < beta {
        return Ok(None);
    }

    context.telemetry.null_move_fail_highs += 1;
    context.telemetry.null_move_verifications += 1;
    context.mode = verification_mode;
    let verification = negamax(
        board,
        history,
        verification_depth,
        ply,
        extensions_used,
        beta - 1,
        beta,
        previous_move,
        &[],
        context,
    );
    context.mode = original_mode;
    context.clear_pv(ply);
    let verification = verification?;
    if verification.score >= beta {
        return Ok(Some(NodeResult {
            score: beta,
            path_dependent: false,
        }));
    }
    Ok(None)
}

struct SearchContext<'a> {
    control: &'a SearchControl,
    table: &'a mut TranspositionTable,
    scoring: EvaluationConfig,
    personality: EvaluationConfig,
    mode: SearchMode,
    telemetry: SearchTelemetry,
    null_move_enabled: bool,
    node_limit: Option<u64>,
    nodes: u64,
    started: Instant,
    pv: Vec<Vec<Move>>,
    hash_pv_depths: Vec<Option<u32>>,
    picker_storage: Vec<MovePickerStorage>,
    ordering: MoveOrdering,
}

impl SearchContext<'_> {
    fn visit_node(&mut self) -> Result<(), Aborted> {
        if self.node_limit_reached()
            || (should_poll_control(self.nodes) && self.control_stop_requested())
        {
            return Err(Aborted);
        }
        self.nodes += 1;
        match self.mode {
            SearchMode::NullProbe => self.telemetry.null_probe_nodes += 1,
            SearchMode::Verification => self.telemetry.null_verification_nodes += 1,
            SearchMode::Normal => {}
        }
        Ok(())
    }

    fn should_stop(&self) -> bool {
        self.node_limit_reached() || self.control_stop_requested()
    }

    fn control_stop_requested(&self) -> bool {
        self.control.is_stopped() || self.control.hard_deadline_reached()
    }

    fn node_limit_reached(&self) -> bool {
        self.node_limit.is_some_and(|limit| self.nodes >= limit)
    }

    fn clear_pv(&mut self, ply: u32) {
        let ply = ply.min(MAX_PLY) as usize;
        self.pv[ply].clear();
        self.hash_pv_depths[ply] = None;
    }

    fn update_pv(&mut self, board: &Board, ply: u32, chess_move: Move) {
        let ply = ply.min(MAX_PLY) as usize;
        if ply < MAX_PLY as usize && self.hash_pv_depths[ply + 1].is_some() {
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            self.resolve_hash_pv(&child, ply as u32 + 1);
        }
        let (current_rows, child_rows) = self.pv.split_at_mut(ply + 1);
        let current = &mut current_rows[ply];
        current.clear();
        current.push(chess_move);
        if let Some(child) = child_rows.first() {
            current.extend_from_slice(child);
        }
        self.hash_pv_depths[ply] = None;
    }

    fn mark_hash_pv(&mut self, bound: Bound, depth: u32, ply: u32) {
        let ply = ply.min(MAX_PLY) as usize;
        self.pv[ply].clear();
        self.hash_pv_depths[ply] = (bound == Bound::Exact).then_some(depth);
    }

    fn resolve_hash_pv(&mut self, board: &Board, ply: u32) {
        let ply = ply.min(MAX_PLY) as usize;
        let Some(depth) = self.hash_pv_depths[ply].take() else {
            return;
        };
        self.table
            .write_principal_variation(board, depth, &mut self.pv[ply]);
    }

    fn pv(&self, ply: u32) -> &[Move] {
        let ply = ply.min(MAX_PLY) as usize;
        debug_assert!(self.hash_pv_depths[ply].is_none());
        &self.pv[ply]
    }

    fn take_picker_storage(&mut self, ply: u32) -> MovePickerStorage {
        std::mem::take(&mut self.picker_storage[ply.min(MAX_PLY) as usize])
    }

    fn recycle_picker_storage(&mut self, ply: u32, mut storage: MovePickerStorage) {
        storage.clear();
        self.picker_storage[ply.min(MAX_PLY) as usize] = storage;
    }

    fn visit_quiescence_node(&mut self) -> Result<(), Aborted> {
        self.visit_node()?;
        self.telemetry.quiescence_nodes += 1;
        Ok(())
    }

    fn legal_move_exists(&mut self, board: &Board) -> bool {
        self.telemetry.legal_move_probes += 1;
        has_legal_move(board)
    }

    fn probe_table(&mut self, key: u64, halfmove_clock: u8) -> Option<Entry> {
        self.telemetry.tt_probes += 1;
        let entry = self.table.probe_key(key, halfmove_clock);
        self.telemetry.tt_hits += u64::from(entry.is_some());
        self.telemetry.tt_hash_moves +=
            u64::from(entry.is_some_and(|entry| entry.best_move().is_some()));
        entry
    }
}

pub(super) fn run<F>(
    position: &Position,
    limits: &SearchLimits,
    control: &SearchControl,
    evaluation: EvaluationConfig,
    move_overhead: Duration,
    table: &mut TranspositionTable,
    mut report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    let scoring = EvaluationConfig::new(0);
    table.start_search(evaluation.aggression());
    let time_budget = allocate_time(position.board().side_to_move(), limits, move_overhead);
    if !control.has_time_budget()
        && let Some(budget) = time_budget
    {
        control.set_time_budget_from_now(budget.soft(), budget.hard());
    }
    let has_time_budget = time_budget.is_some() || control.has_time_budget();

    let root_board = position.board().clone();
    let mut labeled_moves = generate_moves(&root_board)
        .into_iter()
        .map(|chess_move| {
            (
                display_uci_move(&root_board, chess_move).to_string(),
                chess_move,
            )
        })
        .filter(|(move_text, _)| {
            limits.search_moves.is_empty()
                || limits
                    .search_moves
                    .iter()
                    .any(|candidate| candidate == move_text)
        })
        .collect::<Vec<_>>();
    labeled_moves.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    let fallback = labeled_moves
        .first()
        .map(|(move_text, _)| move_text.clone());
    if labeled_moves.is_empty() {
        return SearchResult::from_parts(None, None, SearchTelemetry::default());
    }
    let root_moves = labeled_moves
        .into_iter()
        .map(|(_, chess_move)| chess_move)
        .collect::<Vec<_>>();

    let mut context = SearchContext {
        control,
        table,
        scoring,
        personality: evaluation,
        mode: SearchMode::Normal,
        telemetry: SearchTelemetry::default(),
        null_move_enabled: limits.null_move.unwrap_or(true),
        node_limit: limits.nodes,
        nodes: 0,
        started: Instant::now(),
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
            .collect(),
        hash_pv_depths: vec![None; MAX_PLY as usize + 1],
        picker_storage: (0..=MAX_PLY)
            .map(|_| MovePickerStorage::default())
            .collect(),
        ordering: MoveOrdering::new(),
    };
    let mut history = RepetitionTracker::new(position.hash_history());
    if !context.should_stop()
        && terminal_score(&root_board, &history, 0, false).is_some_and(|result| result.score == 0)
    {
        let info = SearchInfo::new(
            0,
            SearchScore::Centipawns(0),
            0,
            context.started.elapsed(),
            vec![fallback.clone().expect("root moves are non-empty")],
        );
        report(info.clone());
        return SearchResult::from_parts(fallback, Some(info), context.telemetry);
    }
    let mut previous_pv = Vec::new();
    let mut previous_score = None;
    let mut stability = IterationStability::default();
    let mut extension_used = false;
    let mut final_info = None;
    let maximum_depth = maximum_depth(limits, has_time_budget);

    'iterative: for depth in 1..=maximum_depth {
        if context.should_stop() {
            break;
        }

        let iteration_started = Instant::now();
        let mut radius = ASPIRATION_INITIAL;
        let (mut alpha, mut beta) = previous_score
            .filter(|score: &Score| score.abs() < MATE_THRESHOLD)
            .map_or((NEG_INFINITY, POS_INFINITY), |score| {
                aspiration_bounds(score, radius)
            });
        let mut aspiration_searches = 0_u32;
        let iteration = loop {
            let finite_window = alpha != NEG_INFINITY || beta != POS_INFINITY;
            if finite_window {
                context.telemetry.aspiration_attempts += 1;
            }
            let is_research = aspiration_searches > 0;
            aspiration_searches += 1;
            let nodes_before = context.nodes;
            let iteration = search_root(
                &root_board,
                &root_moves,
                &mut history,
                depth,
                (alpha, beta),
                &previous_pv,
                &mut context,
            );
            if is_research {
                context.telemetry.aspiration_research_nodes +=
                    context.nodes.saturating_sub(nodes_before);
            }
            let Ok(iteration) = iteration else {
                break 'iterative;
            };

            if iteration.primary_inside((alpha, beta)) {
                break iteration;
            }
            if finite_window {
                if iteration.primary_score <= alpha {
                    context.telemetry.aspiration_fail_lows += 1;
                } else if iteration.primary_score >= beta {
                    context.telemetry.aspiration_fail_highs += 1;
                }
            }
            if alpha == NEG_INFINITY && beta == POS_INFINITY {
                break iteration;
            }

            radius = radius.saturating_mul(2);
            if iteration.primary_score.abs() >= MATE_THRESHOLD || radius >= POS_INFINITY {
                alpha = NEG_INFINITY;
                beta = POS_INFINITY;
            } else if let Some(score) = previous_score {
                (alpha, beta) = aspiration_bounds(score, radius);
            }
        };
        let iteration_duration = iteration_started.elapsed();

        let is_volatile =
            stability.observe(context.pv(0).first().copied(), iteration.primary_score);
        previous_score = Some(iteration.primary_score);
        previous_pv.clear();
        previous_pv.extend_from_slice(context.pv(0));
        let pv = format_pv(&root_board, &previous_pv);
        let info = SearchInfo::new(
            depth,
            SearchScore::from_internal(iteration.selected.score),
            context.nodes,
            context.started.elapsed(),
            pv,
        );
        report(info.clone());
        let found_mate = matches!(info.score(), SearchScore::Mate(_));
        final_info = Some(info);

        if found_mate || context.should_stop() {
            break;
        }
        match next_iteration_decision(
            control.deadline_window(),
            iteration_duration,
            is_volatile,
            extension_used,
        ) {
            IterationDecision::Stop => break,
            IterationDecision::Continue => {}
            IterationDecision::Extend => extension_used = true,
        }
    }

    let best_move = final_info
        .as_ref()
        .and_then(|info| info.pv().first().cloned())
        .or(fallback);
    SearchResult::from_parts(best_move, final_info, context.telemetry)
}

fn maximum_depth(limits: &SearchLimits, has_deadline: bool) -> u32 {
    let depth_limit = limits.depth.map(|depth| depth.clamp(1, MAX_DEPTH));
    let mate_limit = limits
        .mate
        .map(|moves| moves.saturating_mul(2).clamp(1, MAX_DEPTH));
    match (depth_limit, mate_limit) {
        (Some(depth), Some(mate)) => depth.min(mate),
        (Some(depth), None) => depth,
        (None, Some(mate)) => mate,
        (None, None)
            if limits.nodes.is_some() || has_deadline || limits.infinite || limits.ponder =>
        {
            MAX_DEPTH
        }
        (None, None) => DEFAULT_DEPTH,
    }
}
fn aspiration_bounds(center: Score, radius: Score) -> (Score, Score) {
    (
        center.saturating_sub(radius).max(NEG_INFINITY),
        center.saturating_add(radius).min(POS_INFINITY),
    )
}

fn mate_distance_bounds(ply: u32) -> (Score, Score) {
    (-MATE_SCORE + ply as Score, MATE_SCORE - ply as Score - 1)
}
fn next_search_depth(
    depth: u32,
    in_check: bool,
    extensions_used: u8,
    max_extensions: u8,
) -> (u32, u8) {
    let extend = in_check && extensions_used < max_extensions;
    (
        depth.saturating_sub(1) + u32::from(extend),
        extensions_used + u8::from(extend),
    )
}

fn search_root(
    board: &Board,
    root_moves: &[Move],
    history: &mut RepetitionTracker,
    depth: u32,
    window: (Score, Score),
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<RootSearchResult, Aborted> {
    if context.personality.root_style_margin() == 0 {
        let objective_start_nodes = context.nodes;
        let selected = search_root_conventional(
            board,
            root_moves,
            history,
            depth,
            window,
            previous_pv,
            context,
        );
        context.telemetry.objective_root_nodes +=
            context.nodes.saturating_sub(objective_start_nodes);
        return Ok(RootSearchResult::from_primary(selected?.selected));
    }
    search_root_styled(
        board,
        root_moves,
        history,
        depth,
        window,
        previous_pv,
        context,
    )
}

fn search_root_styled(
    board: &Board,
    root_moves: &[Move],
    history: &mut RepetitionTracker,
    depth: u32,
    window: (Score, Score),
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<RootSearchResult, Aborted> {
    let objective_start_nodes = context.nodes;
    let conventional = search_root_conventional(
        board,
        root_moves,
        history,
        depth,
        window,
        previous_pv,
        context,
    );
    let objective_nodes = context.nodes.saturating_sub(objective_start_nodes);
    context.telemetry.objective_root_nodes += objective_nodes;
    let conventional = conventional?;
    let ConventionalRootResult {
        selected: objective,
        evidence,
    } = conventional;
    let objective_pv = context.pv(0).to_vec();
    let Some(objective_move) = objective_pv.first().copied() else {
        return Ok(RootSearchResult::from_primary(objective));
    };
    if objective.score.abs() >= MATE_THRESHOLD
        || objective.score <= window.0
        || objective.score >= window.1
        || context.should_stop()
    {
        return Ok(RootSearchResult::from_primary(objective));
    }

    let threshold = objective
        .score
        .saturating_sub(context.personality.root_style_margin().min(120));
    let mover = board.side_to_move();
    let root_snapshot = tactical_snapshot(board, mover);
    let mut objective_child = board.clone();
    objective_child.play_unchecked(objective_move);
    let objective_sacrifice = sacrifice_profile(board, &objective_child, mover, &objective_pv);
    let objective_outcome = root_line_outcome(
        board,
        history,
        &objective_pv,
        objective.score,
        objective.path_dependent,
    );
    let objective_sterile =
        sterile_simplification(board, &objective_pv, mover, objective_sacrifice.attack_gain);
    let objective_metadata =
        MoveMetadata::classify_with_child(board, objective_move, &objective_child, true);
    let mut candidates = vec![RootCandidate {
        chess_move: objective_move,
        score: objective.score,
        path_dependent: objective.path_dependent,
        interest: root_interest(
            board,
            &objective_child,
            objective_metadata,
            context.personality,
        ),
        pv: objective_pv.clone(),
        sacrifice: objective_sacrifice,
        outcome: objective_outcome,
        sterile_simplification: objective_sterile,
    }];
    let mut seeds = Vec::with_capacity(root_moves.len().saturating_sub(1));
    for &chess_move in root_moves {
        if chess_move == objective_move {
            continue;
        }
        if evidence
            .iter()
            .find(|entry| entry.chess_move == chess_move)
            .is_some_and(|entry| {
                root_evidence_decision(entry, threshold) == RootEvidenceDecision::Reject
            })
        {
            continue;
        }
        if context.control_stop_requested() {
            context.pv[0].clone_from(&objective_pv);
            return Ok(RootSearchResult::from_primary(objective));
        }
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let metadata = MoveMetadata::classify_with_child(board, chess_move, &child, true);
        let interest = root_interest(board, &child, metadata, context.personality);
        let immediate = tactical_snapshot(&child, mover);
        let offered_cp = exchange_risk_on(&child, mover, chess_move.to);
        let sacrifice_hint =
            sacrifice_hint_score(&root_snapshot, &immediate, offered_cp, metadata.gives_check);
        seeds.push(CandidateSeed {
            chess_move,
            interest,
            sacrifice_hint,
        });
    }
    let alternatives = prioritize_probe_seeds(select_candidate_seeds(seeds));
    let tactical_reserve = alternatives
        .iter()
        .any(|seed| seed.sacrifice_hint >= MIN_SACRIFICE_CP);
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        !board.checkers().is_empty(),
        0,
        context.personality.max_check_extensions(),
    );
    let original_node_limit = context.node_limit;
    let personality_node_limit = styled_root_node_limit(
        context.nodes,
        objective_nodes,
        tactical_reserve,
        original_node_limit,
    );
    context.node_limit = Some(personality_node_limit);
    let personality_start_nodes = context.nodes;
    let mut probe_passers = Vec::new();
    let mut personality_exhausted = false;

    for seed in alternatives {
        if context.should_stop() {
            personality_exhausted = true;
            break;
        }
        if let Some(entry) = evidence
            .iter()
            .find(|entry| entry.chess_move == seed.chess_move)
        {
            match root_evidence_decision(entry, threshold) {
                RootEvidenceDecision::Reject => continue,
                RootEvidenceDecision::Accept => {
                    probe_passers.push(ProbedCandidate {
                        seed,
                        child_pv: entry.child_pv.clone(),
                    });
                    continue;
                }
                RootEvidenceDecision::Probe => {}
            }
        }
        let current_move = HistoryMove::from_board(board, seed.chess_move);
        let mut child = board.clone();
        child.play_unchecked(seed.chess_move);
        history.push_key(repetition_key(&child));
        let probe = negamax(
            &child,
            history,
            child_depth,
            1,
            child_extensions,
            -threshold,
            -threshold + 1,
            Some(current_move),
            &[],
            context,
        );
        history.pop();
        let Ok(probe) = probe else {
            personality_exhausted = true;
            break;
        };
        if -probe.score >= threshold {
            context.resolve_hash_pv(&child, 1);
            probe_passers.push(ProbedCandidate {
                seed,
                child_pv: context.pv(1).to_vec(),
            });
            if probe_passers.len() == STYLED_ROOT_MAX_VERIFICATIONS {
                break;
            }
        }
    }

    for probed in select_verification_candidates(probe_passers) {
        context.telemetry.personality_verifications += 1;
        if context.should_stop() {
            personality_exhausted = true;
            break;
        }
        let seed = probed.seed;
        let current_move = HistoryMove::from_board(board, seed.chess_move);
        let mut child = board.clone();
        child.play_unchecked(seed.chess_move);
        let child_key = repetition_key(&child);
        let verification_alpha = threshold.saturating_sub(1).max(NEG_INFINITY);
        let verification_beta = objective.score.saturating_add(1).min(POS_INFINITY);
        history.push_key(child_key);
        let mut child_result = negamax(
            &child,
            history,
            child_depth,
            1,
            child_extensions,
            -verification_beta,
            -verification_alpha,
            Some(current_move),
            &probed.child_pv,
            context,
        );
        if matches!(&child_result, Ok(result) if -result.score >= verification_beta) {
            child_result = negamax(
                &child,
                history,
                child_depth,
                1,
                child_extensions,
                NEG_INFINITY,
                POS_INFINITY,
                Some(current_move),
                &probed.child_pv,
                context,
            );
        }
        history.pop();
        let Ok(child_result) = child_result else {
            personality_exhausted = true;
            break;
        };
        let mut score = -child_result.score;
        let provisional_margin = if seed.sacrifice_hint >= MIN_SACRIFICE_CP {
            context.personality.root_style_margin().min(120)
        } else {
            candidate_risk_margin(
                context.personality,
                objective.score,
                &SacrificeProfile::default(),
            )
        };
        if !candidate_within_score_guard(score, objective.score, provisional_margin) {
            continue;
        }
        let mut path_dependent = child_result.path_dependent;
        context.resolve_hash_pv(&child, 1);
        let mut verified_child_pv = context.pv(1).to_vec();
        let mut pv = vec![seed.chess_move];
        pv.extend_from_slice(&verified_child_pv);
        let mut sacrifice = sacrifice_profile(board, &child, mover, &pv);

        let should_extend = context.personality.aggression() >= 75
            && seed.sacrifice_hint >= MIN_SACRIFICE_CP
            && is_compensated_sacrifice(&sacrifice)
            && !context.should_stop();
        if should_extend {
            history.push_key(child_key);
            let mut extended = negamax(
                &child,
                history,
                child_depth + 1,
                1,
                child_extensions,
                -verification_beta,
                -verification_alpha,
                Some(current_move),
                &verified_child_pv,
                context,
            );
            if matches!(&extended, Ok(result) if -result.score >= verification_beta) {
                extended = negamax(
                    &child,
                    history,
                    child_depth + 1,
                    1,
                    child_extensions,
                    NEG_INFINITY,
                    POS_INFINITY,
                    Some(current_move),
                    &verified_child_pv,
                    context,
                );
            }
            history.pop();
            match extended {
                Ok(extended) => {
                    score = -extended.score;
                    path_dependent = extended.path_dependent;
                    context.resolve_hash_pv(&child, 1);
                    verified_child_pv = context.pv(1).to_vec();
                    pv.clear();
                    pv.push(seed.chess_move);
                    pv.extend_from_slice(&verified_child_pv);
                    sacrifice = sacrifice_profile(board, &child, mover, &pv);
                }
                Err(_) => {
                    personality_exhausted = true;
                    break;
                }
            }
        }
        let verified_margin =
            candidate_risk_margin(context.personality, objective.score, &sacrifice);
        if !candidate_within_score_guard(score, objective.score, verified_margin) {
            continue;
        }

        let outcome = root_line_outcome(board, history, &pv, score, path_dependent);
        let sterile = sterile_simplification(board, &pv, mover, sacrifice.attack_gain);
        candidates.push(RootCandidate {
            chess_move: seed.chess_move,
            score,
            path_dependent,
            interest: seed.interest,
            pv,
            sacrifice,
            outcome,
            sterile_simplification: sterile,
        });
    }

    if context.nodes >= personality_node_limit {
        personality_exhausted = true;
    }
    context.node_limit = original_node_limit;
    context.telemetry.personality_root_nodes +=
        context.nodes.saturating_sub(personality_start_nodes);
    if context.should_stop() || (personality_exhausted && candidates.len() == 1) {
        context.pv[0].clone_from(&objective_pv);
        return Ok(RootSearchResult::from_primary(objective));
    }

    let selected = choose_styled_candidate(&candidates, 0, context.personality);
    context.pv[0].clone_from(&candidates[selected].pv);
    Ok(RootSearchResult {
        primary_score: objective.score,
        selected: NodeResult {
            score: candidates[selected].score,
            path_dependent: candidates[selected].path_dependent,
        },
    })
}

fn choose_styled_candidate(
    candidates: &[RootCandidate],
    conventional: usize,
    evaluation: EvaluationConfig,
) -> usize {
    let best = candidates[conventional].score;
    let mut selected = conventional;
    for (index, candidate) in candidates.iter().enumerate() {
        let margin = candidate_risk_margin(evaluation, best, &candidate.sacrifice);
        if !candidate_within_score_guard(candidate.score, best, margin) {
            continue;
        }
        let current = &candidates[selected];
        let candidate_interest = selection_interest(candidate, evaluation, best);
        let current_interest = selection_interest(current, evaluation, best);
        if candidate_interest > current_interest
            || (candidate_interest == current_interest && candidate.score > current.score)
            || (candidate_interest == current_interest
                && candidate.score == current.score
                && move_key(candidate.chess_move) < move_key(current.chess_move))
        {
            selected = index;
        }
    }
    selected
}

fn candidate_within_score_guard(candidate: Score, best: Score, margin: Score) -> bool {
    if best >= 0 && candidate < 0 {
        return false;
    }
    if candidate.abs() >= MATE_THRESHOLD || best.abs() >= MATE_THRESHOLD {
        candidate >= best
    } else {
        candidate >= best - margin
    }
}

fn candidate_risk_margin(
    evaluation: EvaluationConfig,
    best_score: Score,
    sacrifice: &SacrificeProfile,
) -> Score {
    let hard_margin = evaluation.root_style_margin().min(120);
    if best_score >= WINNING_ROOT_SCORE {
        return hard_margin.min(WINNING_ROOT_MARGIN_MAX);
    }
    if is_compensated_sacrifice(sacrifice) {
        return hard_margin;
    }
    hard_margin.min(ORDINARY_ROOT_MARGIN_MAX)
}

fn is_compensated_sacrifice(sacrifice: &SacrificeProfile) -> bool {
    sacrifice.state == SacrificeState::Accepted
        && sacrifice.settled_exchange
        && sacrifice.verified_reply
        && sacrifice.offered_cp >= MIN_SACRIFICE_CP
        && sacrifice.accepted_cp >= MIN_SACRIFICE_CP
        && sacrifice.compensation_signals >= 2
        && sacrifice.legal_checks > 0
        && sacrifice.attack_gain > 0
        && sacrifice.position_stable
        && sacrifice.king_danger_delta <= 20
}

fn sacrifice_material(sacrifice: &SacrificeProfile) -> Score {
    match sacrifice.state {
        SacrificeState::Accepted if sacrifice.settled_exchange => sacrifice.accepted_cp,
        SacrificeState::None
        | SacrificeState::Accepted
        | SacrificeState::Declined
        | SacrificeState::Unverified => 0,
    }
}

fn selection_interest(
    candidate: &RootCandidate,
    evaluation: EvaluationConfig,
    best_score: Score,
) -> i64 {
    let mut interest = candidate.interest;
    if is_compensated_sacrifice(&candidate.sacrifice) {
        interest += 1_000_000
            + i64::from(sacrifice_material(&candidate.sacrifice)) * 100
            + i64::from(candidate.sacrifice.compensation_signals) * 10_000
            + i64::from(candidate.sacrifice.legal_checks) * 2_000
            + i64::from(candidate.sacrifice.attack_gain.max(0)) * 100
            + i64::from(candidate.sacrifice.queens_retained) * 5_000
            + (20_i64 - candidate.sacrifice.reply_count.min(20) as i64) * 500
            - i64::from(candidate.sacrifice.king_danger_delta.max(0)) * 100;
    }
    if evaluation.aggression() >= 75 && best_score >= WINNING_ROOT_SCORE {
        if candidate.outcome != RootLineOutcome::Live {
            interest -= i64::from(evaluation.aggression()) * 20_000;
        }
        if candidate.sterile_simplification {
            interest -= i64::from(evaluation.aggression()) * 2_000;
        }
    }
    interest
}

fn root_line_outcome(
    root: &Board,
    history: &RepetitionTracker,
    pv: &[Move],
    score: Score,
    path_dependent: bool,
) -> RootLineOutcome {
    if score == 0 && path_dependent {
        return RootLineOutcome::RepetitionDraw;
    }
    let mut board = root.clone();
    let mut tracker = history.clone();
    for (index, &chess_move) in pv.iter().enumerate() {
        if !board.is_legal(chess_move) {
            return RootLineOutcome::Live;
        }
        board.play_unchecked(chess_move);
        tracker.push_key(repetition_key(&board));
        let no_legal_moves = generate_moves(&board).is_empty();
        if let Some(result) = terminal_score(&board, &tracker, index as u32 + 1, no_legal_moves)
            && result.score == 0
        {
            return if result.path_dependent {
                RootLineOutcome::RepetitionDraw
            } else if index == 0 {
                RootLineOutcome::ImmediateDraw
            } else {
                RootLineOutcome::SearchedDraw
            };
        }
    }
    RootLineOutcome::Live
}

fn sterile_simplification(root: &Board, pv: &[Move], mover: Color, attack_gain: Score) -> bool {
    if pv.len() < 2 || attack_gain > 0 {
        return false;
    }
    let before = style_snapshot(root, mover);
    if before.material_balance.abs() > 200 {
        return false;
    }
    let mut board = root.clone();
    for &chess_move in pv.iter().take(2) {
        if !board.is_legal(chess_move) {
            return false;
        }
        board.play_unchecked(chess_move);
    }
    let after = style_snapshot(&board, mover);
    let equal_trade = (after.material_balance - before.material_balance).abs() <= 50;
    let removed_major_material = total_major_material(root) - total_major_material(&board);
    equal_trade && removed_major_material >= 1_000
}

fn total_major_material(board: &Board) -> Score {
    piece_value(Piece::Queen) * board.pieces(Piece::Queen).len() as Score
        + piece_value(Piece::Rook) * board.pieces(Piece::Rook).len() as Score
}

fn sacrifice_hint_score(
    before: &TacticalSnapshot,
    immediate: &TacticalSnapshot,
    offered_cp: Score,
    gives_check: bool,
) -> Score {
    if offered_cp < MIN_SACRIFICE_CP {
        return 0;
    }
    offered_cp
        + (immediate.style.attack_momentum - before.style.attack_momentum).max(0) * 4
        + immediate.style.coordination * 20
        + Score::from(gives_check) * 150
        - (immediate.style.own_king_danger - before.style.own_king_danger).max(0) * 2
}

fn select_candidate_seeds(seeds: Vec<CandidateSeed>) -> Vec<CandidateSeed> {
    let mut ordinary = seeds.clone();
    ordinary.sort_unstable_by(|left, right| {
        right
            .interest
            .cmp(&left.interest)
            .then_with(|| move_key(left.chess_move).cmp(&move_key(right.chess_move)))
    });
    let mut sacrifices = seeds
        .into_iter()
        .filter(|seed| seed.sacrifice_hint >= MIN_SACRIFICE_CP)
        .collect::<Vec<_>>();
    sacrifices.sort_unstable_by(|left, right| {
        right
            .sacrifice_hint
            .cmp(&left.sacrifice_hint)
            .then_with(|| right.interest.cmp(&left.interest))
            .then_with(|| move_key(left.chess_move).cmp(&move_key(right.chess_move)))
    });

    let mut selected = Vec::with_capacity(6);
    for seed in ordinary.iter().copied().take(3) {
        push_unique_seed(&mut selected, seed);
    }
    for seed in sacrifices.into_iter().take(3) {
        push_unique_seed(&mut selected, seed);
    }
    for seed in ordinary {
        if selected.len() == 6 {
            break;
        }
        push_unique_seed(&mut selected, seed);
    }
    selected
}

fn styled_root_node_limit(
    current_nodes: u64,
    objective_nodes: u64,
    tactical_reserve: bool,
    global_limit: Option<u64>,
) -> u64 {
    let (divisor, maximum) = if tactical_reserve {
        (
            STYLED_ROOT_TACTICAL_BUDGET_DIVISOR,
            STYLED_ROOT_TACTICAL_MAX_NODES,
        )
    } else {
        (STYLED_ROOT_BUDGET_DIVISOR, STYLED_ROOT_MAX_NODES)
    };
    let budget = (objective_nodes / divisor).clamp(STYLED_ROOT_MIN_NODES, maximum);
    let local_limit = current_nodes.saturating_add(budget);
    global_limit.map_or(local_limit, |limit| local_limit.min(limit))
}

fn prioritize_probe_seeds(seeds: Vec<CandidateSeed>) -> Vec<CandidateSeed> {
    let mut ordinary = seeds.clone();
    ordinary.sort_unstable_by(|left, right| {
        right
            .interest
            .cmp(&left.interest)
            .then_with(|| move_key(left.chess_move).cmp(&move_key(right.chess_move)))
    });
    let mut sacrifices = seeds
        .iter()
        .copied()
        .filter(|seed| seed.sacrifice_hint >= MIN_SACRIFICE_CP)
        .collect::<Vec<_>>();
    sacrifices.sort_unstable_by(|left, right| {
        right
            .sacrifice_hint
            .cmp(&left.sacrifice_hint)
            .then_with(|| right.interest.cmp(&left.interest))
            .then_with(|| move_key(left.chess_move).cmp(&move_key(right.chess_move)))
    });

    let mut prioritized = Vec::with_capacity(seeds.len());
    if let Some(seed) = ordinary.first().copied() {
        push_unique_seed(&mut prioritized, seed);
    }
    if let Some(seed) = sacrifices.first().copied() {
        push_unique_seed(&mut prioritized, seed);
    }
    for seed in seeds {
        push_unique_seed(&mut prioritized, seed);
    }
    prioritized
}

fn select_verification_candidates(candidates: Vec<ProbedCandidate>) -> Vec<ProbedCandidate> {
    let mut ordinary = candidates.clone();
    ordinary.sort_unstable_by(|left, right| {
        right
            .seed
            .interest
            .cmp(&left.seed.interest)
            .then_with(|| move_key(left.seed.chess_move).cmp(&move_key(right.seed.chess_move)))
    });
    let mut sacrifices = candidates
        .into_iter()
        .filter(|candidate| candidate.seed.sacrifice_hint >= MIN_SACRIFICE_CP)
        .collect::<Vec<_>>();
    sacrifices.sort_unstable_by(|left, right| {
        right
            .seed
            .sacrifice_hint
            .cmp(&left.seed.sacrifice_hint)
            .then_with(|| right.seed.interest.cmp(&left.seed.interest))
            .then_with(|| move_key(left.seed.chess_move).cmp(&move_key(right.seed.chess_move)))
    });

    let mut selected = Vec::with_capacity(STYLED_ROOT_MAX_VERIFICATIONS);
    if let Some(candidate) = ordinary.first().cloned() {
        push_unique_verification(&mut selected, candidate);
    }
    if let Some(candidate) = sacrifices.first().cloned() {
        push_unique_verification(&mut selected, candidate);
    }
    for candidate in ordinary {
        if selected.len() == STYLED_ROOT_MAX_VERIFICATIONS {
            break;
        }
        push_unique_verification(&mut selected, candidate);
    }
    selected
}

fn push_unique_verification(selected: &mut Vec<ProbedCandidate>, candidate: ProbedCandidate) {
    if !selected
        .iter()
        .any(|entry| entry.seed.chess_move == candidate.seed.chess_move)
    {
        selected.push(candidate);
    }
}

fn push_unique_seed(selected: &mut Vec<CandidateSeed>, seed: CandidateSeed) {
    if !selected
        .iter()
        .any(|candidate| candidate.chess_move == seed.chess_move)
    {
        selected.push(seed);
    }
}

const MIN_SACRIFICE_CP: Score = 80;

fn sacrifice_profile(root: &Board, child: &Board, mover: Color, pv: &[Move]) -> SacrificeProfile {
    let before = tactical_snapshot(root, mover);
    let immediate = tactical_snapshot(child, mover);
    let reply_count = generate_moves(child).len();
    let Some(&root_move) = pv.first().filter(|&&chess_move| root.is_legal(chess_move)) else {
        return SacrificeProfile::default();
    };
    let target = root_move.to;
    let prior_risk = exchange_risk_on(root, mover, target);
    let offered_cp = (exchange_risk_on(child, mover, target) - prior_risk).max(0);
    let Some(&reply) = pv.get(1).filter(|&&reply| child.is_legal(reply)) else {
        return SacrificeProfile {
            state: if offered_cp >= MIN_SACRIFICE_CP {
                SacrificeState::Unverified
            } else {
                SacrificeState::None
            },
            offered_cp,
            remaining_offer_cp: offered_cp,
            reply_count,
            attack_gain: immediate.style.attack_momentum - before.style.attack_momentum,
            king_danger_delta: immediate.style.own_king_danger - before.style.own_king_danger,
            legal_checks: immediate.legal_checks,
            compensation_signals: compensation_signals(&before, &immediate, reply_count),
            queens_retained: immediate.style.mover_queens > 0
                && immediate.style.total_queens >= before.style.total_queens,
            position_stable: immediate.exchange_risk <= before.exchange_risk + offered_cp,
            ..SacrificeProfile::default()
        };
    };

    let reply_accepts_offer = reply.to == target
        && child.color_on(target) == Some(mover)
        && captured_piece(child, reply).is_some();
    let mut reply_board = child.clone();
    reply_board.play_unchecked(reply);
    if !reply_accepts_offer {
        let after = tactical_snapshot(&reply_board, mover);
        return SacrificeProfile {
            state: if offered_cp >= MIN_SACRIFICE_CP {
                SacrificeState::Declined
            } else {
                SacrificeState::None
            },
            offered_cp,
            remaining_offer_cp: exchange_risk_on(&reply_board, mover, target),
            reply_count,
            attack_gain: after.style.attack_momentum - before.style.attack_momentum,
            king_danger_delta: after.style.own_king_danger - before.style.own_king_danger,
            legal_checks: after.legal_checks,
            compensation_signals: compensation_signals(&before, &after, reply_count),
            queens_retained: after.style.mover_queens > 0
                && after.style.total_queens >= before.style.total_queens,
            position_stable: after.exchange_risk <= before.exchange_risk + MIN_SACRIFICE_CP,
            verified_reply: true,
            ..SacrificeProfile::default()
        };
    }

    let outcome = exchange_outcome(&reply_board, mover, target);
    debug_assert_eq!(outcome.target, target);
    let after = tactical_snapshot(&outcome.final_board, mover);
    let accepted_cp = (before.style.material_balance - outcome.material_balance).max(0);
    let state = if outcome.truncated {
        SacrificeState::Unverified
    } else if offered_cp >= MIN_SACRIFICE_CP && accepted_cp >= MIN_SACRIFICE_CP {
        SacrificeState::Accepted
    } else {
        SacrificeState::None
    };

    SacrificeProfile {
        state,
        settled_exchange: !outcome.truncated,
        offered_cp,
        accepted_cp,
        remaining_offer_cp: exchange_risk_on(&outcome.final_board, mover, target),
        reply_count,
        attack_gain: after.style.attack_momentum - before.style.attack_momentum,
        king_danger_delta: after.style.own_king_danger - before.style.own_king_danger,
        legal_checks: after.legal_checks,
        compensation_signals: compensation_signals(&before, &after, reply_count),
        queens_retained: after.style.mover_queens > 0
            && after.style.total_queens >= before.style.total_queens,
        position_stable: after.exchange_risk <= before.exchange_risk + MIN_SACRIFICE_CP,
        verified_reply: !outcome.truncated,
    }
}

fn compensation_signals(
    before: &TacticalSnapshot,
    after: &TacticalSnapshot,
    reply_count: usize,
) -> u8 {
    let mut signals = 0;
    signals += u8::from(after.style.attackers >= 2);
    signals += u8::from(after.style.attacker_variety >= 2);
    signals += u8::from(after.style.coordination > before.style.coordination);
    signals += u8::from(after.style.supported_threats > 0);
    signals += u8::from(after.style.open_lines > before.style.open_lines);
    signals += u8::from(after.style.defender_shortage > 0);
    signals += u8::from(after.style.pawn_breaks > before.style.pawn_breaks);
    signals += u8::from(after.style.attack_momentum > before.style.attack_momentum);
    signals += u8::from(after.legal_checks > 0);
    signals += u8::from(reply_count <= 6);
    signals
}

fn search_root_conventional(
    board: &Board,
    root_moves: &[Move],
    history: &mut RepetitionTracker,
    depth: u32,
    window: (Score, Score),
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<ConventionalRootResult, Aborted> {
    let (mut alpha, beta) = window;
    context.clear_pv(0);
    if context.should_stop() {
        return Err(Aborted);
    }
    let alpha_original = alpha;
    let hash_move = context
        .probe_table(history.current_key(), board.halfmove_clock())
        .and_then(|entry| entry.best_move());
    let preferred = previous_pv.first().copied().or(hash_move);
    let moves = prepare_and_order_root_moves(
        board,
        root_moves.to_vec(),
        preferred,
        &context.ordering,
        context.personality,
    );
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        !board.checkers().is_empty(),
        0,
        context.personality.max_check_extensions(),
    );
    let mut best = NodeResult {
        score: NEG_INFINITY,
        path_dependent: false,
    };
    let collect_evidence = context.personality.root_style_margin() != 0;
    let mut evidence = if collect_evidence {
        Vec::with_capacity(moves.len())
    } else {
        Vec::new()
    };

    for (index, prepared) in moves.into_iter().enumerate() {
        if context.should_stop() {
            return Err(Aborted);
        }

        let chess_move = prepared.metadata.chess_move;
        let current_move = HistoryMove::from_board(board, chess_move);
        let prepared_child = &prepared.child;
        let child_key = repetition_key(prepared_child);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        let move_alpha = alpha;
        history.push_key(child_key);
        let first_window = if index == 0 {
            (-beta, -alpha)
        } else {
            (-alpha - 1, -alpha)
        };
        let mut child_result = negamax(
            prepared_child,
            history,
            child_depth,
            1,
            child_extensions,
            first_window.0,
            first_window.1,
            Some(current_move),
            expected_child_pv,
            context,
        );
        history.pop();
        let mut score = -child_result.as_ref().map_err(|_| Aborted)?.score;

        if index != 0 && score > alpha && score < beta {
            history.push_key(child_key);
            child_result = negamax(
                prepared_child,
                history,
                child_depth,
                1,
                child_extensions,
                -beta,
                -alpha,
                Some(current_move),
                expected_child_pv,
                context,
            );
            history.pop();
            score = -child_result.as_ref().map_err(|_| Aborted)?.score;
        }
        let child_result = child_result?;
        let bound = if score <= move_alpha {
            Bound::Upper
        } else if score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        if collect_evidence {
            let child_pv = if bound == Bound::Exact {
                context.resolve_hash_pv(prepared_child, 1);
                context.pv(1).to_vec()
            } else {
                Vec::new()
            };
            evidence.push(RootMoveEvidence {
                chess_move,
                score,
                bound,
                child_pv,
            });
        }

        if score > best.score {
            best = NodeResult {
                score,
                path_dependent: child_result.path_dependent,
            };
            context.update_pv(board, 0, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }

    if context.mode.writes_tt() && !best.path_dependent {
        let bound = if best.score <= alpha_original {
            Bound::Upper
        } else if best.score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        context.table.store_key(
            history.current_key(),
            board.halfmove_clock(),
            depth,
            0,
            best.score,
            bound,
            context.pv(0).first().copied(),
        );
    }
    Ok(ConventionalRootResult {
        selected: best,
        evidence,
    })
}

#[allow(clippy::too_many_arguments)]
fn negamax(
    board: &Board,
    history: &mut RepetitionTracker,
    depth: u32,
    ply: u32,
    extensions_used: u8,
    mut alpha: Score,
    mut beta: Score,
    previous_move: Option<HistoryMove>,
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    if depth == 0 {
        return quiescence(
            board,
            history,
            ply,
            alpha,
            beta,
            QUIESCENCE_DEPTH,
            context.personality.quiescence_check_budget(),
            None,
            context,
        );
    }

    context.clear_pv(ply);
    context.visit_node()?;
    if draw_state_pending(board, history, context.mode) || ply >= MAX_PLY {
        if let Some(result) = terminal_score_for_mode(
            board,
            history,
            ply,
            !context.legal_move_exists(board),
            context.mode,
        ) {
            return Ok(NodeResult {
                score: result.score,
                path_dependent: result.path_dependent,
            });
        }
        return Ok(NodeResult {
            score: evaluate_with_config(board, context.scoring),
            path_dependent: false,
        });
    }

    let (mate_alpha, mate_beta) = mate_distance_bounds(ply);
    alpha = alpha.max(mate_alpha);
    beta = beta.min(mate_beta);
    let hash_entry = context
        .mode
        .reads_tt()
        .then(|| context.probe_table(history.current_key(), board.halfmove_clock()))
        .flatten();
    let hash_move = hash_entry
        .and_then(|entry| entry.best_move())
        .filter(|&chess_move| board.is_legal(chess_move));
    if alpha >= beta {
        if hash_move.is_none()
            && let Some(result) = terminal_without_legal_moves(board, history, ply, context)
        {
            return Ok(result);
        }
        return Ok(NodeResult {
            score: alpha,
            path_dependent: false,
        });
    }

    let alpha_original = alpha;
    if let Some(entry) = hash_entry.filter(|entry| entry.depth() >= depth) {
        let score = entry.score_at_ply(ply);
        let cutoff = match entry.bound() {
            Bound::Exact => true,
            Bound::Lower => score >= beta,
            Bound::Upper => score <= alpha,
        };
        if cutoff {
            if hash_move.is_none()
                && let Some(result) = terminal_without_legal_moves(board, history, ply, context)
            {
                return Ok(result);
            }
            context.telemetry.tt_cutoffs += 1;
            context.mark_hash_pv(entry.bound(), depth, ply);
            return Ok(NodeResult {
                score,
                path_dependent: false,
            });
        }
    }

    let in_check = !board.checkers().is_empty();
    let pv_node = beta.saturating_sub(alpha) > 1;
    let static_evaluation =
        if static_pruning_allowed(board, depth, alpha, beta, pv_node, context.mode) {
            context.telemetry.static_pruning_attempts += 1;
            Some(evaluate_with_config(board, context.scoring))
        } else {
            None
        };
    if static_evaluation.is_some_and(|evaluation| {
        reverse_futility_cutoff(evaluation, beta, depth, context.personality.aggression())
    }) {
        if hash_move.is_none()
            && let Some(result) = terminal_without_legal_moves(board, history, ply, context)
        {
            return Ok(result);
        }
        context.telemetry.reverse_futility_cutoffs += 1;
        return Ok(NodeResult {
            score: beta,
            path_dependent: false,
        });
    }
    if let Some(result) = verified_null_move_cutoff(
        board,
        history,
        depth,
        ply,
        extensions_used,
        alpha,
        beta,
        static_evaluation,
        previous_move,
        context,
    )? {
        if hash_move.is_none()
            && let Some(terminal) = terminal_without_legal_moves(board, history, ply, context)
        {
            return Ok(terminal);
        }
        context.telemetry.null_move_cutoffs += 1;
        return Ok(result);
    }

    let preferred = previous_pv.first().copied().or(hash_move);
    let picker_storage = context.take_picker_storage(ply);
    let mut picker = MovePicker::new(
        board,
        picker_storage,
        preferred,
        ply,
        previous_move,
        context.personality,
        MovePickerMode::Main,
    );
    let Some(first_move) = picker.next(&context.ordering) else {
        context.recycle_picker_storage(ply, picker.into_storage());
        return Ok(known_terminal_without_legal_moves(
            board,
            history,
            ply,
            context.mode,
        ));
    };
    let mut prefetched_move = Some(first_move);
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        in_check,
        extensions_used,
        context.personality.max_check_extensions(),
    );
    let mut best = NodeResult {
        score: NEG_INFINITY,
        path_dependent: false,
    };
    let mut selective_fail_low = false;

    while let Some((index, metadata)) = prefetched_move
        .take()
        .or_else(|| picker.next(&context.ordering))
    {
        let chess_move = metadata.chess_move;
        let current_move = HistoryMove::from_board(board, chess_move);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        let protected = preferred == Some(chess_move)
            || !expected_child_pv.is_empty()
            || (context.personality.aggression() > 0 && metadata.attacking_pawn_push)
            || context
                .ordering
                .killers(ply)
                .into_iter()
                .flatten()
                .any(|killer| killer == chess_move);
        let history_score = context
            .ordering
            .quiet_history_score(board, chess_move, previous_move);
        if static_evaluation.is_some_and(|evaluation| {
            should_prune_quiet_move(
                depth,
                index,
                metadata,
                protected,
                history_score,
                evaluation,
                alpha,
                context.personality.aggression(),
            )
        }) {
            context.telemetry.futility_pruned_moves += 1;
            selective_fail_low = true;
            picker.record_failed_quiet(metadata);
            continue;
        }
        context.telemetry.lmr_attempts += 1;
        let reduction = late_move_reduction(
            child_depth,
            index,
            metadata,
            protected,
            in_check,
            pv_node,
            history_score,
        );
        if reduction > 0 {
            context.telemetry.lmr_reductions += 1;
        }
        let mut child = board.clone();
        child.play_unchecked_with_piece(chess_move, metadata.attacker);
        let child_key = repetition_key(&child);
        history.push_key(child_key);
        let first_window = if index == 0 {
            (-beta, -alpha)
        } else {
            (-alpha - 1, -alpha)
        };
        let mut child_result = negamax(
            &child,
            history,
            child_depth.saturating_sub(reduction),
            ply + 1,
            child_extensions,
            first_window.0,
            first_window.1,
            Some(current_move),
            expected_child_pv,
            context,
        );
        history.pop();
        let mut score = -child_result.as_ref().map_err(|_| Aborted)?.score;

        if reduced_search_needs_research(reduction, score, alpha) {
            context.telemetry.lmr_researches += 1;
            history.push_key(child_key);
            child_result = negamax(
                &child,
                history,
                child_depth,
                ply + 1,
                child_extensions,
                first_window.0,
                first_window.1,
                Some(current_move),
                expected_child_pv,
                context,
            );
            history.pop();
            score = -child_result.as_ref().map_err(|_| Aborted)?.score;
            if score >= beta {
                context.telemetry.lmr_research_fail_highs += 1;
            }
        } else if reduction > 0 {
            selective_fail_low = true;
        }

        if index != 0 && score > alpha && score < beta {
            history.push_key(child_key);
            child_result = negamax(
                &child,
                history,
                child_depth,
                ply + 1,
                child_extensions,
                -beta,
                -alpha,
                Some(current_move),
                expected_child_pv,
                context,
            );
            history.pop();
            score = -child_result.as_ref().map_err(|_| Aborted)?.score;
        }
        let child_result = child_result?;

        if score > best.score {
            best = NodeResult {
                score,
                path_dependent: child_result.path_dependent,
            };
            context.update_pv(board, ply, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            if context.mode.updates_ordering() && metadata.is_quiet() {
                context.ordering.record_quiet_cutoff(
                    board,
                    previous_move,
                    chess_move,
                    picker.failed_quiets(),
                    ply,
                    depth,
                );
            } else if metadata.facts().captured.is_some() {
                context.telemetry.capture_cutoffs += 1;
                context.telemetry.capture_cutoff_index_sum += index as u64;
                if context.mode.updates_ordering()
                    && metadata.chess_move.promotion.is_none()
                    && !picker.failed_captures().is_empty()
                {
                    context.ordering.record_capture_cutoff(
                        board,
                        metadata,
                        picker.failed_captures(),
                        depth,
                    );
                    context.telemetry.capture_history_updates += 1;
                }
            }
            break;
        }
        picker.record_failed_quiet(metadata);
        picker.record_failed_capture(metadata);
    }

    if context.mode.writes_tt() && !best.path_dependent {
        let bound = if best.score <= alpha_original {
            Bound::Upper
        } else if best.score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        if !selective_fail_low || bound == Bound::Lower {
            context.table.store_key(
                history.current_key(),
                board.halfmove_clock(),
                depth,
                ply,
                best.score,
                bound,
                context.pv(ply).first().copied(),
            );
        }
    }

    context.recycle_picker_storage(ply, picker.into_storage());
    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn quiescence(
    board: &Board,
    history: &mut RepetitionTracker,
    ply: u32,
    mut alpha: Score,
    mut beta: Score,
    remaining: u32,
    check_budget: u8,
    recapture_square: Option<cozy_chess::Square>,
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    context.clear_pv(ply);
    context.visit_quiescence_node()?;
    if draw_state_pending(board, history, context.mode) || ply >= MAX_PLY {
        if let Some(result) = terminal_score_for_mode(
            board,
            history,
            ply,
            !context.legal_move_exists(board),
            context.mode,
        ) {
            return Ok(NodeResult {
                score: result.score,
                path_dependent: result.path_dependent,
            });
        }
        return Ok(NodeResult {
            score: evaluate_with_config(board, context.scoring),
            path_dependent: false,
        });
    }
    let (mate_alpha, mate_beta) = mate_distance_bounds(ply);
    alpha = alpha.max(mate_alpha);
    beta = beta.min(mate_beta);
    if alpha >= beta {
        if let Some(result) = terminal_without_legal_moves(board, history, ply, context) {
            return Ok(result);
        }
        return Ok(NodeResult {
            score: alpha,
            path_dependent: false,
        });
    }

    let in_check = !board.checkers().is_empty();
    if remaining == 0 && !in_check {
        if let Some(result) = terminal_without_legal_moves(board, history, ply, context) {
            return Ok(result);
        }
        return Ok(NodeResult {
            score: evaluate_with_config(board, context.scoring),
            path_dependent: false,
        });
    }

    let stand_pat = if in_check {
        None
    } else {
        Some(evaluate_with_config(board, context.scoring))
    };
    let mut best = NodeResult {
        score: stand_pat.unwrap_or(NEG_INFINITY),
        path_dependent: false,
    };
    if let Some(stand_pat) = stand_pat {
        if stand_pat >= beta {
            if let Some(result) = terminal_without_legal_moves(board, history, ply, context) {
                return Ok(result);
            }
            return Ok(best);
        }
        alpha = alpha.max(stand_pat);
    }
    let picker_storage = context.take_picker_storage(ply);
    let mut picker = MovePicker::new(
        board,
        picker_storage,
        None,
        ply,
        None,
        context.personality,
        MovePickerMode::Quiescence {
            in_check,
            include_quiet_checks: check_budget > 0,
        },
    );
    let Some(first_move) = picker.next(&context.ordering) else {
        context.recycle_picker_storage(ply, picker.into_storage());
        if in_check || !context.legal_move_exists(board) {
            return Ok(known_terminal_without_legal_moves(
                board,
                history,
                ply,
                context.mode,
            ));
        }
        return Ok(best);
    };
    let mut prefetched_move = Some(first_move);

    while let Some((index, metadata)) = prefetched_move
        .take()
        .or_else(|| picker.next(&context.ordering))
    {
        debug_assert!(
            in_check || metadata.is_tactical() || (metadata.is_quiet() && metadata.gives_check)
        );
        if should_prune_quiescence_capture(
            metadata,
            in_check,
            recapture_square,
            context.personality.aggression(),
            stand_pat.unwrap_or(NEG_INFINITY),
            alpha,
        ) {
            continue;
        }
        let chess_move = metadata.chess_move;
        let uses_quiet_check = !in_check && metadata.is_quiet() && metadata.gives_check;
        let next_check_budget = check_budget.saturating_sub(u8::from(uses_quiet_check));
        let mut child = board.clone();
        child.play_unchecked_with_piece(chess_move, metadata.attacker);
        let child_key = repetition_key(&child);
        history.push_key(child_key);
        let child_result = quiescence(
            &child,
            history,
            ply + 1,
            -beta,
            -alpha,
            remaining.saturating_sub(1),
            next_check_budget,
            Some(chess_move.to),
            context,
        );
        history.pop();
        let child_result = child_result?;
        let score = -child_result.score;

        if score > best.score {
            best = NodeResult {
                score,
                path_dependent: child_result.path_dependent,
            };
            context.update_pv(board, ply, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            if metadata.facts().captured.is_some() {
                context.telemetry.capture_cutoffs += 1;
                context.telemetry.capture_cutoff_index_sum += index as u64;
                if context.mode.updates_ordering()
                    && metadata.chess_move.promotion.is_none()
                    && !picker.failed_captures().is_empty()
                {
                    context.ordering.record_capture_cutoff(
                        board,
                        metadata,
                        picker.failed_captures(),
                        remaining.max(1),
                    );
                    context.telemetry.capture_history_updates += 1;
                }
            }
            break;
        }
        picker.record_failed_capture(metadata);
    }

    context.recycle_picker_storage(ply, picker.into_storage());
    Ok(best)
}

fn terminal_score(
    board: &Board,
    history: &RepetitionTracker,
    ply: u32,
    no_legal_moves: bool,
) -> Option<TerminalResult> {
    terminal_score_for_mode(board, history, ply, no_legal_moves, SearchMode::Normal)
}

fn terminal_score_for_mode(
    board: &Board,
    history: &RepetitionTracker,
    ply: u32,
    no_legal_moves: bool,
    mode: SearchMode,
) -> Option<TerminalResult> {
    if no_legal_moves {
        return Some(TerminalResult {
            score: if board.checkers().is_empty() {
                0
            } else {
                -MATE_SCORE + ply as Score
            },
            path_dependent: false,
        });
    }
    if mode.tracks_legal_draws() && history.occurrences(history.current_key()) >= 3 {
        return Some(TerminalResult {
            score: 0,
            path_dependent: true,
        });
    }
    if (mode.tracks_legal_draws() && board.halfmove_clock() >= 100) || is_dead_material(board) {
        return Some(TerminalResult {
            score: 0,
            path_dependent: false,
        });
    }
    None
}

fn draw_state_pending(board: &Board, history: &RepetitionTracker, mode: SearchMode) -> bool {
    (mode.tracks_legal_draws()
        && (history.occurrences(history.current_key()) >= 3 || board.halfmove_clock() >= 100))
        || is_dead_material(board)
}

fn terminal_without_legal_moves(
    board: &Board,
    history: &RepetitionTracker,
    ply: u32,
    context: &mut SearchContext<'_>,
) -> Option<NodeResult> {
    if context.legal_move_exists(board) {
        None
    } else {
        Some(known_terminal_without_legal_moves(
            board,
            history,
            ply,
            context.mode,
        ))
    }
}

fn known_terminal_without_legal_moves(
    board: &Board,
    history: &RepetitionTracker,
    ply: u32,
    mode: SearchMode,
) -> NodeResult {
    let result = terminal_score_for_mode(board, history, ply, true, mode)
        .expect("a position without legal moves is terminal");
    NodeResult {
        score: result.score,
        path_dependent: result.path_dependent,
    }
}

fn is_dead_material(board: &Board) -> bool {
    if !board.pieces(Piece::Pawn).is_empty()
        || !board.pieces(Piece::Rook).is_empty()
        || !board.pieces(Piece::Queen).is_empty()
    {
        return false;
    }

    let knights = board.pieces(Piece::Knight);
    let bishops = board.pieces(Piece::Bishop);
    if knights.len() + bishops.len() <= 1 {
        return true;
    }

    knights.is_empty()
        && ((bishops & BitBoard::DARK_SQUARES) == bishops
            || (bishops & BitBoard::LIGHT_SQUARES) == bishops)
}

fn move_gives_check(board: &Board, chess_move: Move, moved: Piece) -> bool {
    let color = board.side_to_move();
    let enemy = !color;
    let enemy_king = board.king(enemy);
    let from = chess_move.from.bitboard();
    let to = chess_move.to.bitboard();
    let mut pieces = [BitBoard::EMPTY; Piece::NUM];
    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ] {
        pieces[piece as usize] = board.colored_pieces(color, piece);
    }
    let mut enemy_pieces = board.colors(enemy);
    let is_castling = board.colors(color).has(chess_move.to);

    if is_castling {
        let back_rank = Rank::First.relative_to(color);
        let (king_file, rook_file) = if chess_move.from.file() < chess_move.to.file() {
            (cozy_chess::File::G, cozy_chess::File::F)
        } else {
            (cozy_chess::File::C, cozy_chess::File::D)
        };
        pieces[Piece::King as usize] &= !from;
        pieces[Piece::King as usize] |= Square::new(king_file, back_rank).bitboard();
        pieces[Piece::Rook as usize] &= !to;
        pieces[Piece::Rook as usize] |= Square::new(rook_file, back_rank).bitboard();
    } else {
        pieces[moved as usize] &= !from;
        let placed = chess_move.promotion.unwrap_or(moved);
        pieces[placed as usize] |= to;
        enemy_pieces &= !to;

        let is_en_passant = moved == Piece::Pawn
            && chess_move.from.file() != chess_move.to.file()
            && board.piece_on(chess_move.to).is_none();
        if is_en_passant {
            let victim = Square::new(chess_move.to.file(), Rank::Fifth.relative_to(color));
            enemy_pieces &= !victim.bitboard();
        }
    }

    let friendly = pieces
        .iter()
        .copied()
        .fold(BitBoard::EMPTY, |occupied, piece| occupied | piece);
    let occupied = friendly | enemy_pieces;
    !(get_pawn_attacks(enemy_king, enemy) & pieces[Piece::Pawn as usize]).is_empty()
        || !(get_knight_moves(enemy_king) & pieces[Piece::Knight as usize]).is_empty()
        || !(get_king_moves(enemy_king) & pieces[Piece::King as usize]).is_empty()
        || !(get_bishop_moves(enemy_king, occupied)
            & (pieces[Piece::Bishop as usize] | pieces[Piece::Queen as usize]))
            .is_empty()
        || !(get_rook_moves(enemy_king, occupied)
            & (pieces[Piece::Rook as usize] | pieces[Piece::Queen as usize]))
            .is_empty()
}

fn has_legal_move(board: &Board) -> bool {
    let mut found = false;
    board.generate_moves(|piece_moves| {
        found = piece_moves.into_iter().next().is_some();
        found
    });
    found
}

fn generate_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

#[cfg(test)]
fn order_moves(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ply: u32,
    ordering: &MoveOrdering,
) -> Vec<Move> {
    order_moves_with_evaluation(
        board,
        moves,
        preferred,
        ply,
        ordering,
        EvaluationConfig::new(0),
    )
}

#[cfg(test)]
fn order_moves_with_evaluation(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ply: u32,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<Move> {
    order_move_metadata(board, moves, preferred, ply, None, ordering, evaluation)
        .into_iter()
        .map(|metadata| metadata.chess_move)
        .collect()
}

#[cfg(test)]
fn order_move_metadata(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<MoveMetadata> {
    prepare_and_order_moves(board, moves, preferred, ply, previous, ordering, evaluation)
        .into_iter()
        .map(|prepared| prepared.metadata)
        .collect()
}

#[cfg(test)]
fn prepare_and_order_moves(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<PreparedMove> {
    let prepared = moves
        .into_iter()
        .map(|chess_move| PreparedMove::new(board, chess_move, evaluation.aggression() > 0))
        .collect();
    order_prepared_moves(
        board, prepared, preferred, ply, previous, ordering, evaluation,
    )
}

#[cfg(test)]
fn order_prepared_moves(
    board: &Board,
    mut moves: Vec<PreparedMove>,
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<PreparedMove> {
    order_prepared_moves_in_place(
        board, &mut moves, preferred, ply, previous, ordering, evaluation,
    );
    moves
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_generated_moves_into(
    board: &Board,
    moves: &mut Vec<SearchMove>,
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
    mut retain: impl FnMut(MoveMetadata) -> bool,
) {
    debug_assert!(moves.is_empty());
    let compute_see = evaluation.aggression() > 0;
    board.generate_moves(|piece_moves| {
        for chess_move in piece_moves {
            let metadata = MoveMetadata::classify_for_search(board, chess_move, compute_see);
            if retain(metadata) {
                moves.push(SearchMove {
                    metadata,
                    order_score: 0,
                });
            }
        }
        false
    });
    order_search_moves_in_place(board, moves, preferred, ply, previous, ordering, evaluation);
}

#[cfg(test)]
fn order_prepared_moves_in_place(
    board: &Board,
    moves: &mut [PreparedMove],
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) {
    for prepared in &mut *moves {
        prepared.order_score = move_order_score(
            board,
            prepared.metadata,
            preferred,
            ply,
            previous,
            ordering,
            evaluation,
        );
    }
    moves.sort_unstable_by(|left, right| {
        right.order_score.cmp(&left.order_score).then_with(|| {
            move_key(left.metadata.chess_move).cmp(&move_key(right.metadata.chess_move))
        })
    });
}

#[allow(clippy::too_many_arguments)]
fn order_search_moves_in_place(
    board: &Board,
    moves: &mut [SearchMove],
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) {
    for search_move in &mut *moves {
        search_move.order_score = move_order_score(
            board,
            search_move.metadata,
            preferred,
            ply,
            previous,
            ordering,
            evaluation,
        );
    }
    moves.sort_unstable_by(|left, right| {
        right.order_score.cmp(&left.order_score).then_with(|| {
            move_key(left.metadata.chess_move).cmp(&move_key(right.metadata.chess_move))
        })
    });
}

#[cfg(test)]
fn order_root_moves(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<Move> {
    prepare_and_order_root_moves(board, moves, preferred, ordering, evaluation)
        .into_iter()
        .map(|prepared| prepared.metadata.chess_move)
        .collect()
}

fn prepare_and_order_root_moves(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<PreparedMove> {
    let mover = board.side_to_move();
    let mut prepared = moves
        .into_iter()
        .map(|chess_move| PreparedMove::new(board, chess_move, evaluation.aggression() > 0))
        .collect::<Vec<_>>();
    for candidate in &mut prepared {
        candidate.order_score = move_order_score(
            board,
            candidate.metadata,
            preferred,
            0,
            None,
            ordering,
            evaluation,
        );
        if evaluation.aggression() > 0 {
            candidate.root_complexity = root_complexity_bonus(&candidate.child, mover, evaluation);
        }
    }
    prepared.sort_unstable_by(|left, right| {
        right
            .order_score
            .cmp(&left.order_score)
            .then_with(|| right.root_complexity.cmp(&left.root_complexity))
            .then_with(|| {
                move_key(left.metadata.chess_move).cmp(&move_key(right.metadata.chess_move))
            })
    });
    prepared
}

fn move_order_score(
    board: &Board,
    metadata: MoveMetadata,
    preferred: Option<Move>,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> i64 {
    let chess_move = metadata.chess_move;
    if preferred == Some(chess_move) {
        return 6_000_000;
    }

    if let Some(score) = promotion_order_score(chess_move) {
        return score;
    }

    let capture_history = ordering.capture_history_score(board.side_to_move(), metadata.facts());
    if let Some(score) =
        capture_order_score(metadata.facts(), metadata.see, evaluation, capture_history)
    {
        return score;
    }

    quiet_order_score(board, metadata, ply, previous, ordering, evaluation)
}

fn promotion_order_score(chess_move: Move) -> Option<i64> {
    chess_move
        .promotion
        .map(|promotion| 5_000_000 + i64::from(piece_value(promotion)) * 32)
}

fn capture_order_score(
    facts: MoveFacts,
    see: Option<Score>,
    evaluation: EvaluationConfig,
    capture_history: i32,
) -> Option<i64> {
    let captured_value = ordering_piece_value(facts.captured?);
    let attacker_value = ordering_piece_value(facts.attacker);
    let exchange = i64::from(captured_value) * 32 - i64::from(attacker_value);
    // Retain full influence through the default profile, then taper it to zero.
    let capture_history_scale = i64::from(100_u8.saturating_sub(evaluation.aggression())) * 4;
    let capture_history = i64::from(capture_history) * capture_history_scale.min(100) / 100;
    if evaluation.aggression() > 0
        && let Some(see_score) = see
    {
        let see = i64::from(see_score) * 64;
        return Some(if see_score >= 0 {
            4_000_000 + see + exchange + capture_history
        } else {
            1_000_000 + see + exchange + capture_history
        });
    }
    Some(if captured_value >= attacker_value {
        4_000_000 + exchange + capture_history
    } else {
        1_000_000 + exchange + capture_history
    })
}

fn quiet_order_score(
    board: &Board,
    metadata: MoveMetadata,
    ply: u32,
    previous: Option<HistoryMove>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> i64 {
    let chess_move = metadata.chess_move;
    let history = i64::from(ordering.quiet_history_score(board, chess_move, previous));
    let forcing = forcing_order_bonus(metadata, evaluation);
    if forcing > 0 {
        return 2_000_000 + history + forcing;
    }

    let killers = ordering.killers(ply);
    if killers[0] == Some(chess_move) {
        return 3_000_000;
    }
    if killers[1] == Some(chess_move) {
        return 2_900_000;
    }

    2_000_000 + history
}

fn root_interest(
    board: &Board,
    child: &Board,
    metadata: MoveMetadata,
    evaluation: EvaluationConfig,
) -> i64 {
    let chess_move = metadata.chess_move;
    let mover = board.side_to_move();
    let mut interest = i64::from(root_complexity_bonus(child, mover, evaluation)) * 10;
    interest += i64::from(metadata.gives_check) * 120;
    interest += i64::from(metadata.attacking_pawn_push) * 40;

    let queen_home = match mover {
        Color::White => Square::D1,
        Color::Black => Square::D8,
    };
    let advanced = match mover {
        Color::White => chess_move.to.rank() as i32 >= Rank::Fourth as i32,
        Color::Black => chess_move.to.rank() as i32 <= Rank::Fifth as i32,
    };
    if metadata.attacker == Piece::Queen
        && chess_move.from == queen_home
        && advanced
        && matches!(chess_move.to.file(), File::A | File::B | File::C)
    {
        interest -= i64::from(evaluation.aggression()) * 120;
    }

    interest += chess_move
        .promotion
        .map_or(0, |piece| i64::from(piece_value(piece)) / 5);
    interest += i64::from(child.pieces(Piece::Queen).len()) * 20;
    interest += i64::from(total_non_pawn_material(child)) / 100;
    interest += i64::from(metadata.captured.is_none()) * 15;
    interest
}

fn total_non_pawn_material(board: &Board) -> Score {
    [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen]
        .into_iter()
        .map(|piece| piece_value(piece) * board.pieces(piece).len() as Score)
        .sum()
}

fn forcing_order_bonus(metadata: MoveMetadata, evaluation: EvaluationConfig) -> i64 {
    let aggression = i64::from(evaluation.aggression());
    if aggression == 0 {
        return 0;
    }
    if metadata.gives_check {
        return 800_000 + aggression * 3_000;
    }
    if metadata.attacking_pawn_push {
        return aggression * 1_500;
    }
    0
}

fn is_attacking_pawn_push(board: &Board, chess_move: Move) -> bool {
    if board.piece_on(chess_move.from) != Some(Piece::Pawn) {
        return false;
    }
    let color = board.side_to_move();
    let enemy_king = board.king(!color);
    let near_king = (chess_move.to.file() as i32 - enemy_king.file() as i32).abs() <= 1;
    let advanced = if color == Color::White {
        chess_move.to.rank() as i32 >= 4
    } else {
        chess_move.to.rank() as i32 <= 3
    };
    near_king && advanced
}

fn move_key(chess_move: Move) -> u32 {
    let promotion = chess_move.promotion.map_or(0, |piece| piece as u32 + 1);
    (((chess_move.from as u32 * 64) + chess_move.to as u32) * 8) + promotion
}

fn ordering_piece_value(piece: Piece) -> Score {
    if piece == Piece::King {
        MATE_SCORE
    } else {
        piece_value(piece)
    }
}

fn captured_piece(board: &Board, chess_move: Move) -> Option<Piece> {
    if board.color_on(chess_move.to) == Some(!board.side_to_move()) {
        return board.piece_on(chess_move.to);
    }
    if board.piece_on(chess_move.from) == Some(Piece::Pawn)
        && chess_move.from.file() != chess_move.to.file()
        && board.piece_on(chess_move.to).is_none()
    {
        return Some(Piece::Pawn);
    }
    None
}

fn format_pv(root: &Board, pv: &[Move]) -> Vec<String> {
    let mut board = root.clone();
    pv.iter()
        .map(|&chess_move| {
            let move_text = display_uci_move(&board, chess_move).to_string();
            board.play_unchecked(chess_move);
            move_text
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        HistoryMove, MATE_SCORE, MoveOrdering, RepetitionTracker, generate_moves, order_moves,
        terminal_score,
    };
    use crate::engine::Position;
    use cozy_chess::{Move, Piece, Square};

    #[test]
    fn repetition_tracker_pushes_and_pops_in_constant_time() {
        let position = Position::default();
        let mut tracker = RepetitionTracker::new(position.hash_history());

        let key = super::repetition_key(position.board());
        assert_eq!(tracker.occurrences(key), 1);
        tracker.push_key(key);
        assert_eq!(tracker.occurrences(key), 2);
        tracker.pop();
        assert_eq!(tracker.occurrences(key), 1);
    }

    #[test]
    fn legal_move_probe_matches_full_generation() {
        let positions = [
            Position::default(),
            Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap(),
            Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap(),
        ];

        for position in positions {
            assert_eq!(
                super::has_legal_move(position.board()),
                !generate_moves(position.board()).is_empty(),
                "{position}",
            );
        }
    }

    #[test]
    fn root_evidence_only_skips_proven_candidates() {
        let evidence = |score, bound| super::RootMoveEvidence {
            chess_move: Move {
                from: Square::A1,
                to: Square::A2,
                promotion: None,
            },
            score,
            bound,
            child_pv: Vec::new(),
        };

        assert_eq!(
            super::root_evidence_decision(&evidence(9, super::Bound::Exact), 10),
            super::RootEvidenceDecision::Reject,
        );
        assert_eq!(
            super::root_evidence_decision(&evidence(10, super::Bound::Exact), 10),
            super::RootEvidenceDecision::Accept,
        );
        assert_eq!(
            super::root_evidence_decision(&evidence(9, super::Bound::Upper), 10),
            super::RootEvidenceDecision::Reject,
        );
        assert_eq!(
            super::root_evidence_decision(&evidence(10, super::Bound::Upper), 10),
            super::RootEvidenceDecision::Probe,
        );
        assert_eq!(
            super::root_evidence_decision(&evidence(10, super::Bound::Lower), 10),
            super::RootEvidenceDecision::Accept,
        );
        assert_eq!(
            super::root_evidence_decision(&evidence(9, super::Bound::Lower), 10),
            super::RootEvidenceDecision::Probe,
        );
    }

    #[test]
    fn repetition_draws_are_marked_as_path_dependent() {
        let position = Position::default();
        let key = position.hash_history()[0];
        let tracker = RepetitionTracker::new(&[key, key, key]);

        let result = terminal_score(position.board(), &tracker, 0, false).unwrap();

        assert_eq!(result.score, 0);
        assert!(result.path_dependent);
    }

    #[test]
    fn rule_fifty_draws_are_board_state_dependent() {
        let position = Position::from_fen("7k/8/8/8/8/8/R7/K7 w - - 100 51").unwrap();
        let tracker = RepetitionTracker::new(position.hash_history());

        let result = terminal_score(position.board(), &tracker, 0, false).unwrap();

        assert_eq!(result.score, 0);
        assert!(!result.path_dependent);
    }

    #[test]
    fn checkmate_precedes_rule_fifty() {
        let position = Position::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 100 51").unwrap();
        let tracker = RepetitionTracker::new(position.hash_history());

        let result = terminal_score(
            position.board(),
            &tracker,
            7,
            generate_moves(position.board()).is_empty(),
        )
        .unwrap();

        assert_eq!(result.score, -MATE_SCORE + 7);
        assert!(!result.path_dependent);
    }
    fn find_move(position: &Position, notation: &str) -> cozy_chess::Move {
        position
            .search_moves()
            .into_iter()
            .find(|&chess_move| position.format_search_move(chess_move) == notation)
            .unwrap_or_else(|| panic!("{notation} is not legal in {position}"))
    }

    #[test]
    fn move_ordering_prefers_hash_moves_and_is_otherwise_numeric() {
        let position = Position::default();
        let ordering = MoveOrdering::new();
        let preferred = find_move(&position, "e2e4");

        let ordered = order_moves(
            position.board(),
            position.search_moves(),
            Some(preferred),
            0,
            &ordering,
        );
        assert_eq!(ordered[0], preferred);

        let numeric = order_moves(
            position.board(),
            position.search_moves(),
            None,
            0,
            &ordering,
        );
        assert_eq!(position.format_search_move(numeric[0]), "b1a3");
    }
    #[test]
    fn prepared_moves_match_direct_board_play() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p1NQ/2B1P3/2NP4/PPP2PPP/R4RK1 w - - 0 10",
            "4k3/8/8/8/8/8/4q3/4R1K1 w - - 0 1",
        ] {
            let board = fen.parse::<cozy_chess::Board>().unwrap();
            for chess_move in super::generate_moves(&board) {
                let prepared = super::PreparedMove::new(&board, chess_move, true);
                let mut direct = board.clone();
                direct.play_unchecked(chess_move);
                assert_eq!(prepared.child, direct, "prepared child for {chess_move}");
            }
        }
    }

    #[test]
    fn lazy_move_metadata_matches_played_children() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "4k3/8/8/3pP3/8/8/8/4R1K1 w - d6 0 1",
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            "4k3/8/8/8/8/8/4B3/4R1K1 w - - 0 1",
        ] {
            let board = fen.parse::<cozy_chess::Board>().unwrap();
            for chess_move in super::generate_moves(&board) {
                let lazy = super::MoveMetadata::classify_for_search(&board, chess_move, true);
                let eager = super::PreparedMove::new(&board, chess_move, true).metadata;
                assert_eq!(lazy, eager, "metadata for {chess_move} in {board}");
            }
        }
    }

    #[test]
    fn staged_move_facts_complete_to_the_existing_metadata() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "4k3/8/8/3pP3/8/8/8/4R1K1 w - d6 0 1",
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            "4k3/8/8/8/8/8/4B3/4R1K1 w - - 0 1",
        ] {
            let board = fen.parse::<cozy_chess::Board>().unwrap();
            for chess_move in super::generate_moves(&board) {
                let facts = super::MoveFacts::classify(&board, chess_move);
                let staged = facts.search_metadata(&board, facts.see(&board, true));
                let complete = super::MoveMetadata::classify_for_search(&board, chess_move, true);

                assert_eq!(staged, complete, "metadata for {chess_move} in {board}");
                assert_eq!(staged.facts(), facts, "facts for {chess_move} in {board}");
            }
        }
    }

    #[test]
    fn staged_picker_matches_eager_non_root_ordering() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "4k3/8/8/3pP3/8/8/8/4R1K1 w - d6 0 1",
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            "4k3/8/8/8/8/8/4q3/4R1K1 w - - 0 1",
        ] {
            let board = fen.parse::<cozy_chess::Board>().unwrap();
            let legal_moves = super::generate_moves(&board);
            let preferred = legal_moves.first().copied();
            for aggression in [0, 100] {
                let evaluation = super::EvaluationConfig::new(aggression);
                for preferred in [None, preferred] {
                    let ordering = super::MoveOrdering::new();
                    let mut eager = Vec::new();
                    super::prepare_generated_moves_into(
                        &board,
                        &mut eager,
                        preferred,
                        0,
                        None,
                        &ordering,
                        evaluation,
                        |_| true,
                    );
                    let mut picker = super::MovePicker::new(
                        &board,
                        super::MovePickerStorage::default(),
                        preferred,
                        0,
                        None,
                        evaluation,
                        super::MovePickerMode::Main,
                    );
                    let mut picked = Vec::new();
                    while let Some((index, metadata)) = picker.next(&ordering) {
                        assert_eq!(index, picked.len());
                        picked.push(metadata);
                    }

                    assert_eq!(
                        picked
                            .iter()
                            .map(|metadata| metadata.chess_move)
                            .collect::<Vec<_>>(),
                        eager
                            .iter()
                            .map(|search_move| search_move.metadata.chess_move)
                            .collect::<Vec<_>>(),
                        "order for aggression {aggression} in {fen}",
                    );
                    for (picked, eager) in picked.iter().zip(&eager) {
                        if Some(picked.chess_move) != preferred {
                            assert_eq!(*picked, eager.metadata);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn preferred_picker_move_defers_all_generation_and_see() {
        let board = cozy_chess::Board::default();
        let preferred: Move = "e2e4".parse().unwrap();
        let ordering = super::MoveOrdering::new();
        let mut picker = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            Some(preferred),
            0,
            None,
            super::EvaluationConfig::new(100),
            super::MovePickerMode::Main,
        );

        assert_eq!(picker.next(&ordering).unwrap().1.chess_move, preferred);
        assert_eq!(
            picker.work(),
            super::MovePickerWork {
                check_detections: 1,
                ..super::MovePickerWork::default()
            },
        );
    }

    #[test]
    fn tactical_picker_move_defers_quiet_generation_and_sorting() {
        let board = "4k3/8/8/8/8/8/4q3/4R1K1 w - - 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();
        let capture: Move = "e1e2".parse().unwrap();
        let ordering = super::MoveOrdering::new();
        let mut picker = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            None,
            0,
            None,
            super::EvaluationConfig::new(100),
            super::MovePickerMode::Main,
        );

        assert_eq!(picker.next(&ordering).unwrap().1.chess_move, capture);
        assert_eq!(
            picker.work(),
            super::MovePickerWork {
                tactical_generations: 1,
                check_detections: 1,
                ..super::MovePickerWork::default()
            },
        );
    }

    #[test]
    fn killer_picker_move_defers_quiet_generation_and_is_not_duplicated() {
        let board = cozy_chess::Board::default();
        let killer: Move = "e2e4".parse().unwrap();
        let mut ordering = super::MoveOrdering::new();
        ordering.record_quiet_cutoff(&board, None, killer, &[], 0, 4);
        let mut picker = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            None,
            0,
            None,
            super::EvaluationConfig::new(100),
            super::MovePickerMode::Main,
        );

        let (_, metadata) = picker.next(&ordering).unwrap();
        assert_eq!(metadata.chess_move, killer);
        assert_eq!(
            picker.work(),
            super::MovePickerWork {
                tactical_generations: 1,
                check_detections: 1,
                ..super::MovePickerWork::default()
            },
        );
        picker.record_failed_quiet(metadata);

        let mut picked = vec![metadata.chess_move];
        while let Some((_, metadata)) = picker.next(&ordering) {
            picked.push(metadata.chess_move);
        }
        assert_eq!(
            picked
                .iter()
                .filter(|&&chess_move| chess_move == killer)
                .count(),
            1
        );
        assert_eq!(picker.work().quiet_generations, 1);
        assert_eq!(picked.len(), super::generate_moves(&board).len());
    }

    #[test]
    fn quiescence_picker_skips_or_filters_the_quiet_stage() {
        let board = "7k/8/8/8/8/8/4Q3/4K3 w - - 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();
        let ordering = super::MoveOrdering::new();
        let evaluation = super::EvaluationConfig::new(100);
        let mut tactical_only = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            None,
            0,
            None,
            evaluation,
            super::MovePickerMode::Quiescence {
                in_check: false,
                include_quiet_checks: false,
            },
        );

        assert!(tactical_only.next(&ordering).is_none());
        assert_eq!(
            tactical_only.work(),
            super::MovePickerWork {
                tactical_generations: 1,
                ..super::MovePickerWork::default()
            },
        );

        let quiet_check: Move = "e2e8".parse().unwrap();
        let mut with_checks = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            None,
            0,
            None,
            evaluation,
            super::MovePickerMode::Quiescence {
                in_check: false,
                include_quiet_checks: true,
            },
        );
        let mut picked = Vec::new();
        while let Some((_, metadata)) = with_checks.next(&ordering) {
            assert!(metadata.is_quiet());
            assert!(metadata.gives_check);
            picked.push(metadata.chess_move);
        }

        assert!(picked.contains(&quiet_check));
        assert_eq!(with_checks.work().tactical_generations, 1);
        assert_eq!(with_checks.work().quiet_generations, 1);
        assert_eq!(with_checks.work().quiet_sorts, 1);
        assert!(with_checks.work().check_detections > picked.len());
    }

    #[test]
    fn quiescence_evasion_picker_emits_every_legal_move() {
        let board = "4k3/8/8/8/8/8/4r3/4K3 w - - 0 1"
            .parse::<cozy_chess::Board>()
            .unwrap();
        assert!(!board.checkers().is_empty());
        let ordering = super::MoveOrdering::new();
        let mut picker = super::MovePicker::new(
            &board,
            super::MovePickerStorage::default(),
            None,
            4,
            None,
            super::EvaluationConfig::new(100),
            super::MovePickerMode::Quiescence {
                in_check: true,
                include_quiet_checks: false,
            },
        );
        let mut picked = Vec::new();
        while let Some((_, metadata)) = picker.next(&ordering) {
            picked.push(metadata.chess_move);
        }
        let mut legal = super::generate_moves(&board);
        picked.sort_unstable_by_key(|chess_move| super::move_key(*chess_move));
        legal.sort_unstable_by_key(|chess_move| super::move_key(*chess_move));

        assert_eq!(picked, legal);
        assert_eq!(picker.work().tactical_generations, 1);
        assert_eq!(picker.work().quiet_generations, 1);
    }
    #[test]
    fn static_pruning_requires_stable_non_pv_nodes() {
        let board = cozy_chess::Board::default();
        assert!(super::static_pruning_allowed(
            &board,
            4,
            0,
            1,
            false,
            super::SearchMode::Normal,
        ));
        assert!(!super::static_pruning_allowed(
            &board,
            5,
            0,
            1,
            false,
            super::SearchMode::Normal,
        ));
        assert!(!super::static_pruning_allowed(
            &board,
            4,
            0,
            2,
            true,
            super::SearchMode::Normal,
        ));
        assert!(!super::static_pruning_allowed(
            &board,
            4,
            super::MATE_SCORE - 2,
            super::MATE_SCORE - 1,
            false,
            super::SearchMode::Normal,
        ));

        let rule_fifty = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 80 41"
            .parse()
            .unwrap();
        assert!(!super::static_pruning_allowed(
            &rule_fifty,
            4,
            0,
            1,
            false,
            super::SearchMode::Normal,
        ));
        let pawn_ending = "6k1/5ppp/8/8/6P1/8/5P1P/6K1 w - - 0 1".parse().unwrap();
        assert!(!super::static_pruning_allowed(
            &pawn_ending,
            4,
            0,
            1,
            false,
            super::SearchMode::Normal,
        ));
    }

    #[test]
    fn static_bounds_keep_tactical_and_priority_moves() {
        assert!(super::reverse_futility_cutoff(800, 100, 2, 0));
        assert!(!super::reverse_futility_cutoff(479, 100, 2, 0));

        let position = Position::default();
        let metadata = position
            .search_moves()
            .into_iter()
            .map(|chess_move| super::MoveMetadata::classify(position.board(), chess_move))
            .find(|metadata| {
                metadata.is_quiet()
                    && !metadata.gives_check
                    && !metadata.attacking_pawn_push
                    && !metadata.castling
                    && !metadata.king_zone_move
            })
            .unwrap();
        assert!(super::should_prune_quiet_move(
            1, 2, metadata, false, 0, -500, 0, 0,
        ));
        assert!(!super::should_prune_quiet_move(
            1, 0, metadata, false, 0, -500, 0, 0,
        ));
        assert!(!super::should_prune_quiet_move(
            1, 2, metadata, true, 0, -500, 0, 0,
        ));
    }

    #[test]
    fn root_complexity_orders_equally_ranked_quiet_moves() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let quiet_moves = generate_moves(position.board())
            .into_iter()
            .filter(|&chess_move| {
                super::MoveMetadata::classify(position.board(), chess_move).is_quiet()
            })
            .collect::<Vec<_>>();
        let config = super::EvaluationConfig::default();
        let complexity = |chess_move| {
            let mut child = position.board().clone();
            child.play_unchecked(chess_move);
            super::root_complexity_bonus(&child, position.board().side_to_move(), config)
        };
        let minimum = quiet_moves.iter().copied().map(complexity).min().unwrap();
        let maximum = quiet_moves.iter().copied().map(complexity).max().unwrap();

        let ordered = super::order_root_moves(
            position.board(),
            quiet_moves,
            None,
            &MoveOrdering::new(),
            config,
        );

        assert!(maximum > minimum);
        assert_eq!(complexity(ordered[0]), maximum);
    }

    #[test]
    fn quiet_cutoffs_reward_winners_and_penalize_failed_quiets() {
        let position = Position::default();
        let winner = find_move(&position, "d2d4");
        let failed = find_move(&position, "e2e4");
        let failed_metadata = super::MoveMetadata::classify(position.board(), failed);
        let mut ordering = MoveOrdering::new();

        ordering.record_quiet_cutoff(position.board(), None, winner, &[failed_metadata], 3, 8);
        let ordered = order_moves(
            position.board(),
            position.search_moves(),
            None,
            3,
            &ordering,
        );

        assert_eq!(ordered[0], winner);
        assert!(ordering.history_score(position.board().side_to_move(), winner) > 0);
        assert!(ordering.history_score(position.board().side_to_move(), failed) < 0);
        for _ in 0..100 {
            ordering.record_quiet_cutoff(position.board(), None, winner, &[failed_metadata], 3, 64);
        }
        assert!(
            ordering.history_score(position.board().side_to_move(), winner) <= super::HISTORY_MAX
        );
        assert!(
            ordering.history_score(position.board().side_to_move(), failed) >= -super::HISTORY_MAX
        );
    }
    #[test]
    fn continuation_history_distinguishes_predecessors() {
        let position = Position::default();
        let winner = find_move(&position, "d2d4");
        let failed = find_move(&position, "e2e4");
        let failed_metadata = super::MoveMetadata::classify(position.board(), failed);
        let predecessor = HistoryMove {
            piece: Piece::Knight,
            to: Square::F3,
        };
        let unrelated = HistoryMove {
            piece: Piece::Knight,
            to: Square::C3,
        };
        let current = HistoryMove::from_board(position.board(), winner);
        let failed_current = HistoryMove::from_board(position.board(), failed);
        let mut ordering = MoveOrdering::new();

        ordering.record_quiet_cutoff(
            position.board(),
            Some(predecessor),
            winner,
            &[failed_metadata],
            3,
            8,
        );

        assert!(ordering.continuation_score(predecessor, current) > 0);
        assert!(ordering.continuation_score(predecessor, failed_current) < 0);
        assert_eq!(ordering.continuation_score(unrelated, current), 0);
        assert!(
            ordering.quiet_history_score(position.board(), winner, Some(predecessor))
                > ordering.quiet_history_score(position.board(), winner, Some(unrelated))
        );
        ordering.update_continuation(unrelated, current, -super::HISTORY_MAX);
        assert_eq!(
            ordering.quiet_history_score(position.board(), winner, Some(unrelated)),
            ordering.history_score(position.board().side_to_move(), winner),
        );
        assert_eq!(
            ordering.quiet_history_score(position.board(), winner, None),
            ordering.history_score(position.board().side_to_move(), winner),
        );
    }

    #[test]
    fn capture_history_rewards_cutoffs_without_crossing_see_tiers() {
        let position = Position::from_fen("k7/8/8/8/3pRp2/8/8/K7 w - - 0 1").unwrap();
        let winner = find_move(&position, "e4d4");
        let failed = find_move(&position, "e4f4");
        let winner = super::MoveMetadata::classify(position.board(), winner);
        let failed = super::MoveMetadata::classify(position.board(), failed);
        let color = position.board().side_to_move();
        let mut ordering = super::MoveOrdering::new();

        for _ in 0..32 {
            ordering.record_capture_cutoff(position.board(), winner, &[failed], 8);
        }

        let winner_history = ordering.capture_history_score(color, winner.facts());
        let failed_history = ordering.capture_history_score(color, failed.facts());
        let opposite_history = ordering.capture_history_score(!color, winner.facts());
        assert!(winner_history > 0);
        assert!(failed_history < 0);
        assert_eq!(opposite_history, 0);
        assert!(winner_history <= super::HISTORY_MAX);
        assert!(failed_history >= -super::HISTORY_MAX);

        let evaluation = super::EvaluationConfig::new(75);
        let good =
            super::capture_order_score(winner.facts(), Some(0), evaluation, -super::HISTORY_MAX)
                .unwrap();
        let losing =
            super::capture_order_score(winner.facts(), Some(-1), evaluation, super::HISTORY_MAX)
                .unwrap();
        assert!(good > losing);
    }

    #[test]
    fn equal_captures_are_ordered_before_losing_captures() {
        let position = Position::from_fen("4k3/8/8/3q4/8/2p2p2/3P1Q2/4K3 w - - 0 1").unwrap();
        let equal_capture = find_move(&position, "d2c3");
        let losing_capture = find_move(&position, "f2f3");
        let ordered = order_moves(
            position.board(),
            vec![losing_capture, equal_capture],
            None,
            4,
            &MoveOrdering::new(),
        );

        assert_eq!(ordered, vec![equal_capture, losing_capture]);
    }

    #[test]
    fn see_orders_equivalent_captures_by_the_settled_exchange() {
        let position = Position::from_fen("3r3k/8/8/3p4/p7/8/8/3Q3K w - - 0 1").unwrap();
        let safe_capture = find_move(&position, "d1a4");
        let poisoned_capture = find_move(&position, "d1d5");
        let safe = super::MoveMetadata::classify(position.board(), safe_capture);
        let poisoned = super::MoveMetadata::classify(position.board(), poisoned_capture);
        let ordered = super::order_moves_with_evaluation(
            position.board(),
            vec![poisoned_capture, safe_capture],
            None,
            4,
            &MoveOrdering::new(),
            super::EvaluationConfig::new(100),
        );

        assert!(safe.see.is_some_and(|see| see > 0));
        assert!(poisoned.see.is_some_and(|see| see < 0));
        assert_eq!(ordered, vec![safe_capture, poisoned_capture]);
    }

    #[test]
    fn qsearch_see_pruning_preserves_forcing_and_recapture_cases() {
        let position = Position::from_fen("3r3k/8/8/3p4/8/8/8/3Q3K w - - 0 1").unwrap();
        let poisoned_capture = find_move(&position, "d1d5");
        let metadata = super::MoveMetadata::classify(position.board(), poisoned_capture);

        assert!(metadata.see.is_some_and(|see| see < 0));
        assert!(!super::should_prune_quiescence_capture(
            metadata, false, None, 100, 0, 300,
        ));
        assert!(!super::should_prune_quiescence_capture(
            metadata,
            false,
            Some(cozy_chess::Square::E5),
            0,
            0,
            300,
        ));
        assert!(super::should_prune_quiescence_capture(
            metadata,
            false,
            Some(cozy_chess::Square::E5),
            100,
            0,
            300,
        ));
        assert!(!super::should_prune_quiescence_capture(
            metadata,
            false,
            Some(poisoned_capture.to),
            100,
            0,
            300,
        ));
        assert!(!super::should_prune_quiescence_capture(
            metadata,
            true,
            Some(cozy_chess::Square::E5),
            100,
            0,
            300,
        ));

        for forcing in [
            super::MoveMetadata {
                gives_check: true,
                ..metadata
            },
            super::MoveMetadata {
                king_zone_move: true,
                ..metadata
            },
            super::MoveMetadata {
                chess_move: Move {
                    promotion: Some(Piece::Queen),
                    ..metadata.chess_move
                },
                ..metadata
            },
        ] {
            assert!(!super::should_prune_quiescence_capture(
                forcing,
                false,
                Some(cozy_chess::Square::E5),
                100,
                0,
                300,
            ));
        }

        let aggressive = super::MoveMetadata {
            attacking_pawn_push: true,
            ..metadata
        };
        assert!(!super::should_prune_quiescence_capture(
            aggressive,
            false,
            Some(cozy_chess::Square::E5),
            100,
            0,
            300,
        ));
    }
    #[test]
    fn aspiration_and_mate_windows_are_bounded() {
        assert_eq!(super::aspiration_bounds(100, 50), (50, 150));
        assert_eq!(
            super::aspiration_bounds(super::POS_INFINITY, 50),
            (super::POS_INFINITY - 50, super::POS_INFINITY),
        );
        assert_eq!(
            super::mate_distance_bounds(7),
            (-super::MATE_SCORE + 7, super::MATE_SCORE - 8),
        );
    }
    #[test]
    fn styled_scores_do_not_reopen_a_satisfied_primary_window() {
        let result = super::RootSearchResult {
            primary_score: 100,
            selected: super::NodeResult {
                score: 20,
                path_dependent: false,
            },
        };

        assert!(result.primary_inside((50, 150)));
        assert!(!(50..150).contains(&result.selected.score));
    }

    #[test]
    fn iteration_stability_holds_time_after_best_move_or_score_swings() {
        let position = Position::default();
        let e4 = find_move(&position, "e2e4");
        let d4 = find_move(&position, "d2d4");
        let mut stability = super::IterationStability::default();

        assert!(!stability.observe(Some(e4), 0));
        assert!(!stability.observe(Some(e4), 10));
        assert!(stability.observe(Some(d4), 15));
        assert!(stability.observe(Some(d4), 20));
        assert!(!stability.observe(Some(d4), 25));
        assert!(stability.observe(Some(d4), 80));
    }
    #[test]
    fn control_polling_has_a_bounded_interval() {
        let interval = super::CONTROL_POLL_INTERVAL_NODES;

        assert!(super::should_poll_control(0));
        assert!(!super::should_poll_control(1));
        assert!(!super::should_poll_control(interval - 1));
        assert!(super::should_poll_control(interval));
        assert!(!super::should_poll_control(interval + 1));
        assert!(super::should_poll_control(interval * 2));
    }
    #[test]
    fn iteration_forecasts_respect_soft_and_hard_windows() {
        use std::time::Duration;

        use super::{DeadlineWindow, IterationDecision};

        let iteration = Duration::from_millis(10);
        let ample_soft_time = DeadlineWindow {
            soft: Duration::from_millis(25),
            hard: Duration::from_millis(100),
        };
        assert_eq!(
            super::next_iteration_decision(Some(ample_soft_time), iteration, false, false),
            IterationDecision::Continue
        );
        assert_eq!(
            super::next_iteration_decision(None, iteration, false, false),
            IterationDecision::Continue
        );

        let extension_time = DeadlineWindow {
            soft: Duration::from_millis(24),
            hard: Duration::from_millis(25),
        };
        assert_eq!(
            super::next_iteration_decision(Some(extension_time), iteration, true, false),
            IterationDecision::Extend
        );
        assert_eq!(
            super::next_iteration_decision(Some(extension_time), iteration, false, false),
            IterationDecision::Stop
        );
        assert_eq!(
            super::next_iteration_decision(Some(extension_time), iteration, true, true),
            IterationDecision::Stop
        );

        let fixed_time = DeadlineWindow {
            soft: Duration::from_millis(24),
            hard: Duration::from_millis(24),
        };
        assert_eq!(
            super::next_iteration_decision(Some(fixed_time), iteration, true, false),
            IterationDecision::Stop
        );
    }
    #[test]
    fn check_extensions_are_capped_per_line() {
        let quiet = super::EvaluationConfig::new(0);
        let aggressive = super::EvaluationConfig::new(100);

        assert_eq!(quiet.max_check_extensions(), 2);
        assert_eq!(quiet.quiescence_check_budget(), 1);
        assert_eq!(aggressive.max_check_extensions(), 4);
        assert_eq!(aggressive.quiescence_check_budget(), 3);
        assert_eq!(
            super::next_search_depth(4, false, 0, aggressive.max_check_extensions()),
            (3, 0),
        );
        assert_eq!(
            super::next_search_depth(4, true, 0, aggressive.max_check_extensions()),
            (4, 1),
        );
        assert_eq!(
            super::next_search_depth(
                4,
                true,
                aggressive.max_check_extensions(),
                aggressive.max_check_extensions()
            ),
            (3, aggressive.max_check_extensions()),
        );
    }

    #[test]
    fn null_move_policy_rejects_unsafe_search_states() {
        let rich = Position::default();
        let rich_static =
            super::evaluate_with_config(rich.board(), super::EvaluationConfig::new(0));
        assert_eq!(
            super::null_move_block(
                rich.board(),
                4,
                rich_static,
                false,
                rich_static,
                super::SearchMode::Normal,
            ),
            None,
        );
        assert_eq!(
            super::null_move_block(
                rich.board(),
                4,
                rich_static,
                false,
                rich_static,
                super::SearchMode::NullProbe,
            ),
            Some(super::NullMoveBlock::Mode),
        );
        assert_eq!(
            super::null_move_block(
                rich.board(),
                3,
                rich_static,
                false,
                rich_static,
                super::SearchMode::Normal,
            ),
            Some(super::NullMoveBlock::Depth),
        );
        assert_eq!(
            super::null_move_block(
                rich.board(),
                4,
                rich_static,
                true,
                rich_static,
                super::SearchMode::Normal,
            ),
            Some(super::NullMoveBlock::PvNode),
        );
        let checked = Position::from_fen("4k3/8/8/8/8/8/4r3/3QK3 w - - 0 1").unwrap();
        assert_eq!(
            super::null_move_block(checked.board(), 4, 0, false, 0, super::SearchMode::Normal,),
            Some(super::NullMoveBlock::InCheck),
        );
        let halfmove = Position::from_fen("4k3/8/8/8/8/8/Q7/4K3 w - - 99 50").unwrap();
        assert_eq!(
            super::null_move_block(
                halfmove.board(),
                4,
                0,
                false,
                100,
                super::SearchMode::Normal,
            ),
            Some(super::NullMoveBlock::RuleFifty),
        );
        let pawns = Position::from_fen("8/8/8/8/8/2k5/2p5/2K5 w - - 0 1").unwrap();
        assert_eq!(
            super::null_move_block(pawns.board(), 4, 0, false, 100, super::SearchMode::Normal,),
            Some(super::NullMoveBlock::Material),
        );
    }

    #[test]
    fn null_move_reduction_is_bounded_by_remaining_depth() {
        assert_eq!(super::null_move_reduction(1), 0);
        assert_eq!(super::null_move_reduction(4), 3);
        assert_eq!(super::null_move_reduction(8), 4);
        assert_eq!(super::null_move_reduction(20), 7);
    }

    #[test]
    fn synthetic_null_mode_ignores_legal_history_draws_only() {
        let position = Position::from_fen("4k3/8/8/8/8/8/Q7/4K3 w - - 100 50").unwrap();
        let mut tracker = RepetitionTracker::new(position.hash_history());
        tracker.push_key(super::repetition_key(position.board()));
        tracker.push_key(super::repetition_key(position.board()));

        assert!(
            super::terminal_score_for_mode(
                position.board(),
                &tracker,
                0,
                false,
                super::SearchMode::Normal,
            )
            .is_some()
        );
        assert!(
            super::terminal_score_for_mode(
                position.board(),
                &tracker,
                0,
                false,
                super::SearchMode::Verification,
            )
            .is_some()
        );
        assert!(
            super::terminal_score_for_mode(
                position.board(),
                &tracker,
                0,
                false,
                super::SearchMode::NullProbe,
            )
            .is_none()
        );
        assert!(!super::SearchMode::NullProbe.reads_tt());
        assert!(!super::SearchMode::NullProbe.writes_tt());
        assert!(!super::SearchMode::NullProbe.updates_ordering());
        assert!(!super::SearchMode::NullProbe.allows_null());
        assert!(super::SearchMode::Verification.tracks_legal_draws());
        assert!(super::SearchMode::Verification.writes_tt());
        assert!(!super::SearchMode::Verification.allows_null());
    }

    #[test]
    fn late_move_reductions_protect_tactics_and_use_bounded_history() {
        let position = Position::default();
        let quiet_move = find_move(&position, "a2a3");
        let metadata = super::MoveMetadata::classify(position.board(), quiet_move);

        assert_eq!(
            super::late_move_reduction(3, 3, metadata, false, false, false, 0),
            1,
        );
        assert_eq!(
            super::late_move_reduction(6, 7, metadata, false, false, false, 0),
            2,
        );
        assert_eq!(
            super::late_move_reduction(8, 12, metadata, false, false, false, 0),
            3,
        );
        assert_eq!(
            super::late_move_reduction(
                6,
                7,
                metadata,
                false,
                false,
                false,
                super::LMR_HISTORY_THRESHOLD,
            ),
            1,
        );
        assert_eq!(
            super::late_move_reduction(
                6,
                7,
                metadata,
                false,
                false,
                false,
                -super::LMR_HISTORY_THRESHOLD,
            ),
            3,
        );
        assert_eq!(
            super::late_move_reduction(2, 3, metadata, false, false, false, 0),
            0,
        );
        assert_eq!(
            super::late_move_reduction(3, 2, metadata, false, false, false, 0),
            0,
        );
        assert_eq!(
            super::late_move_reduction(3, 3, metadata, true, false, false, 0),
            0,
        );
        assert_eq!(
            super::late_move_reduction(3, 3, metadata, false, true, false, 0),
            0,
        );
        assert_eq!(
            super::late_move_reduction(6, 7, metadata, false, false, true, 0),
            0,
        );

        for forcing in [
            super::MoveMetadata {
                gives_check: true,
                ..metadata
            },
            super::MoveMetadata {
                castling: true,
                ..metadata
            },
            super::MoveMetadata {
                king_zone_move: true,
                ..metadata
            },
        ] {
            assert_eq!(
                super::late_move_reduction(6, 7, forcing, false, false, false, 0),
                0,
            );
            assert_eq!(
                super::late_move_reduction(6, 7, forcing, false, false, true, 0),
                0,
            );
        }

        let capture = super::MoveMetadata {
            captured: Some(Piece::Pawn),
            ..metadata
        };
        assert_eq!(
            super::late_move_reduction(6, 7, capture, false, false, false, 0),
            0,
        );

        let attacking_push = super::MoveMetadata {
            attacking_pawn_push: true,
            ..metadata
        };
        assert_eq!(
            super::late_move_reduction(6, 7, attacking_push, true, false, false, 0),
            0,
        );
        assert_eq!(
            super::late_move_reduction(6, 7, attacking_push, false, false, false, 0),
            2,
        );
    }

    #[test]
    fn reduced_alpha_raises_require_a_full_depth_research() {
        assert!(super::reduced_search_needs_research(1, 11, 10));
        assert!(!super::reduced_search_needs_research(1, 10, 10));
        assert!(!super::reduced_search_needs_research(0, 11, 10));
    }

    #[test]
    fn aggressive_ordering_prioritizes_quiet_checks() {
        let position = Position::from_fen("7k/8/8/8/8/8/4Q3/4K3 w - - 0 1").unwrap();
        let ordered = super::order_moves_with_evaluation(
            position.board(),
            generate_moves(position.board()),
            None,
            0,
            &MoveOrdering::new(),
            super::EvaluationConfig::new(100),
        );
        let first = super::MoveMetadata::classify(position.board(), ordered[0]);

        assert!(first.gives_check);
        assert!(first.is_quiet());
    }

    #[test]
    fn root_style_margin_scales_from_zero_to_a_pawn() {
        assert_eq!(super::EvaluationConfig::new(0).root_style_margin(), 0);
        assert_eq!(super::EvaluationConfig::new(50).root_style_margin(), 30);
        assert_eq!(super::EvaluationConfig::new(100).root_style_margin(), 120);
    }

    #[test]
    fn sacrifice_profile_marks_material_accepted_after_the_best_reply() {
        let position = Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap();
        let root_move: Move = "c4f7".parse().unwrap();
        let reply: Move = "g8f7".parse().unwrap();
        assert!(position.board().is_legal(root_move));
        let mut child = position.board().clone();
        child.play_unchecked(root_move);
        assert!(child.is_legal(reply));

        let profile = super::sacrifice_profile(
            position.board(),
            &child,
            position.board().side_to_move(),
            &[root_move, reply],
        );

        assert_eq!(profile.state, super::SacrificeState::Accepted);
        assert!(profile.settled_exchange);
        assert!(profile.accepted_cp >= 200);
        assert!(profile.verified_reply);
    }

    #[test]
    fn sacrifice_profile_distinguishes_declined_and_unverified_offers() {
        let position = Position::from_fen("4k3/8/p7/8/2B5/8/8/4K3 w - - 0 1").unwrap();
        let root_move: Move = "c4b5".parse().unwrap();
        let reply: Move = "e8f8".parse().unwrap();
        assert!(position.board().is_legal(root_move));
        let mut child = position.board().clone();
        child.play_unchecked(root_move);
        assert!(child.is_legal(reply));

        let declined = super::sacrifice_profile(
            position.board(),
            &child,
            position.board().side_to_move(),
            &[root_move, reply],
        );
        let unverified = super::sacrifice_profile(
            position.board(),
            &child,
            position.board().side_to_move(),
            &[root_move],
        );

        assert_eq!(declined.state, super::SacrificeState::Declined);
        assert_eq!(declined.offered_cp, 330);
        assert_eq!(declined.accepted_cp, 0);
        assert!(declined.remaining_offer_cp >= 300);
        assert!(declined.verified_reply);
        assert!(!declined.settled_exchange);
        assert_eq!(unverified.state, super::SacrificeState::Unverified);
        assert!(!unverified.settled_exchange);
        assert!(!unverified.verified_reply);
    }

    #[test]
    fn styled_root_choice_stays_inside_the_margin_and_preserves_mates() {
        let position = Position::default();
        let conventional_move = find_move(&position, "e2e4");
        let exciting_move = find_move(&position, "g1f3");
        let mut candidates = vec![
            super::RootCandidate {
                chess_move: conventional_move,
                score: 50,
                path_dependent: false,
                interest: 10,
                pv: vec![conventional_move],
                sacrifice: super::SacrificeProfile::default(),
                outcome: super::RootLineOutcome::Live,
                sterile_simplification: false,
            },
            super::RootCandidate {
                chess_move: exciting_move,
                score: 24,
                path_dependent: false,
                interest: 100,
                pv: vec![exciting_move],
                sacrifice: super::SacrificeProfile::default(),
                outcome: super::RootLineOutcome::Live,
                sterile_simplification: false,
            },
        ];

        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            1
        );
        candidates[1].score = 23;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            0
        );
        candidates[0].score = super::MATE_SCORE - 1;
        candidates[1].score = super::MATE_SCORE - 2;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            0
        );
    }

    #[test]
    fn objective_guard_compares_mate_sign_and_distance() {
        assert!(super::candidate_within_score_guard(
            super::MATE_SCORE - 1,
            super::MATE_SCORE - 2,
            120,
        ));
        assert!(!super::candidate_within_score_guard(
            super::MATE_SCORE - 3,
            super::MATE_SCORE - 2,
            120,
        ));
        assert!(super::candidate_within_score_guard(
            -super::MATE_SCORE + 3,
            -super::MATE_SCORE + 2,
            120,
        ));
        assert!(!super::candidate_within_score_guard(
            -super::MATE_SCORE + 1,
            -super::MATE_SCORE + 2,
            120,
        ));
        assert!(super::candidate_within_score_guard(
            super::MATE_SCORE - 4,
            100,
            120,
        ));
        assert!(!super::candidate_within_score_guard(
            -super::MATE_SCORE + 4,
            100,
            120,
        ));
        assert!(!super::candidate_within_score_guard(-1, 0, 30));
        assert!(super::candidate_within_score_guard(-20, -10, 30));
    }

    #[test]
    fn sacrifice_profile_nets_immediate_recaptures() {
        let position = Position::from_fen(
            "r1bq1rk1/ppp2ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 8",
        )
        .unwrap();
        let root_move: Move = "c1e3".parse().unwrap();
        let reply: Move = "c5e3".parse().unwrap();
        assert!(position.board().is_legal(root_move));
        let mut child = position.board().clone();
        child.play_unchecked(root_move);
        assert!(child.is_legal(reply));

        let profile = super::sacrifice_profile(
            position.board(),
            &child,
            position.board().side_to_move(),
            &[root_move, reply],
        );

        assert_eq!(profile.state, super::SacrificeState::None);
        assert_eq!(profile.accepted_cp, 0);
    }

    #[test]
    fn root_risk_margin_tracks_sacrifice_and_position_context() {
        let sacrifice = super::SacrificeProfile {
            state: super::SacrificeState::Accepted,
            settled_exchange: true,
            offered_cp: 330,
            accepted_cp: 330,
            attack_gain: 10,
            legal_checks: 1,
            compensation_signals: 3,
            position_stable: true,
            verified_reply: true,
            ..super::SacrificeProfile::default()
        };

        for best_score in [-200, 0] {
            assert_eq!(
                super::candidate_risk_margin(
                    super::EvaluationConfig::new(100),
                    best_score,
                    &sacrifice,
                ),
                120,
            );
        }
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(100), 400, &sacrifice),
            20,
        );

        let ordinary = super::SacrificeProfile::default();
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(100), -200, &ordinary),
            26,
        );
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(100), 0, &ordinary),
            26,
        );
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(100), 400, &ordinary),
            20,
        );
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(50), 0, &ordinary),
            26,
        );
        assert_eq!(
            super::candidate_risk_margin(super::EvaluationConfig::new(50), 0, &sacrifice),
            30,
        );
    }

    #[test]
    fn verified_sacrifice_cannot_cross_the_hard_root_margin() {
        let position = Position::default();
        let conventional_move = find_move(&position, "e2e4");
        let sacrifice_move = find_move(&position, "g1f3");
        let sacrifice = super::SacrificeProfile {
            state: super::SacrificeState::Accepted,
            settled_exchange: true,
            offered_cp: 330,
            accepted_cp: 330,
            attack_gain: 10,
            legal_checks: 1,
            compensation_signals: 3,
            position_stable: true,
            verified_reply: true,
            ..super::SacrificeProfile::default()
        };
        let mut candidates = vec![
            super::RootCandidate {
                chess_move: conventional_move,
                score: 50,
                path_dependent: false,
                interest: 10,
                pv: vec![conventional_move],
                sacrifice: super::SacrificeProfile::default(),
                outcome: super::RootLineOutcome::Live,
                sterile_simplification: false,
            },
            super::RootCandidate {
                chess_move: sacrifice_move,
                score: -300,
                path_dependent: false,
                interest: 0,
                pv: vec![sacrifice_move],
                sacrifice,
                outcome: super::RootLineOutcome::Live,
                sterile_simplification: false,
            },
        ];

        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            0,
        );
        candidates[1].score = 0;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            1,
        );
        candidates[1].score = -1;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            0,
        );
        candidates[1].score = 0;
        candidates[1].sacrifice.king_danger_delta = 21;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, super::EvaluationConfig::new(100)),
            0,
        );
    }

    #[test]
    fn verification_pool_keeps_sacrifice_hints() {
        let position = Position::default();
        let moves = position.search_moves();
        let seeds = moves
            .iter()
            .copied()
            .take(9)
            .enumerate()
            .map(|(index, chess_move)| super::CandidateSeed {
                chess_move,
                interest: 1_000 - index as i64,
                sacrifice_hint: if index == 8 { 300 } else { 0 },
            })
            .collect::<Vec<_>>();
        let hinted = seeds[8].chess_move;

        let selected = super::select_candidate_seeds(seeds);

        assert_eq!(selected.len(), 6);
        assert!(selected.iter().any(|seed| seed.chess_move == hinted));
    }

    #[test]
    fn styled_root_budget_is_bounded_and_respects_the_global_limit() {
        assert_eq!(super::styled_root_node_limit(1_000, 4, false, None), 1_256);
        assert_eq!(
            super::styled_root_node_limit(1_000, 400, false, None),
            1_256,
        );
        assert_eq!(
            super::styled_root_node_limit(1_000, 5_000, false, None),
            2_000,
        );
        assert_eq!(
            super::styled_root_node_limit(1_000, 100_000, false, None),
            3_048,
        );
        assert_eq!(
            super::styled_root_node_limit(1_000, 5_000, true, None),
            2_666,
        );
        assert_eq!(
            super::styled_root_node_limit(1_000, 100_000, true, None),
            5_096,
        );
        assert_eq!(
            super::styled_root_node_limit(1_000, 100_000, true, Some(1_500)),
            1_500,
        );
    }

    #[test]
    fn full_verification_keeps_one_high_interest_and_one_sacrifice_seed() {
        let position = Position::default();
        let moves = position.search_moves();
        let candidates = moves
            .iter()
            .copied()
            .take(5)
            .enumerate()
            .map(|(index, chess_move)| super::ProbedCandidate {
                seed: super::CandidateSeed {
                    chess_move,
                    interest: 1_000 - index as i64,
                    sacrifice_hint: if index == 4 { 300 } else { 0 },
                },
                child_pv: Vec::new(),
            })
            .collect::<Vec<_>>();
        let ordinary = candidates[0].seed.chess_move;
        let sacrifice = candidates[4].seed.chess_move;

        let prioritized = super::prioritize_probe_seeds(
            candidates.iter().map(|candidate| candidate.seed).collect(),
        );
        assert_eq!(prioritized[0].chess_move, ordinary);
        assert_eq!(prioritized[1].chess_move, sacrifice);
        let selected = super::select_verification_candidates(candidates);

        assert_eq!(selected.len(), 2);
        assert!(
            selected
                .iter()
                .any(|candidate| candidate.seed.chess_move == ordinary)
        );
        assert!(
            selected
                .iter()
                .any(|candidate| candidate.seed.chess_move == sacrifice)
        );
    }

    #[test]
    fn high_aggression_does_not_trade_a_draw_for_a_negative_score() {
        let position = Position::default();
        let draw_move = find_move(&position, "e2e4");
        let live_move = find_move(&position, "g1f3");
        let mut candidates = vec![
            super::RootCandidate {
                chess_move: draw_move,
                score: 0,
                path_dependent: false,
                interest: 1_000,
                pv: vec![draw_move],
                sacrifice: super::SacrificeProfile::default(),
                outcome: super::RootLineOutcome::ImmediateDraw,
                sterile_simplification: false,
            },
            super::RootCandidate {
                chess_move: live_move,
                score: -30,
                path_dependent: false,
                interest: 2_000,
                pv: vec![live_move],
                sacrifice: super::SacrificeProfile::default(),
                outcome: super::RootLineOutcome::Live,
                sterile_simplification: false,
            },
        ];

        let evaluation = super::EvaluationConfig::new(100);
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, evaluation),
            0
        );
        candidates[1].score = 0;
        assert_eq!(
            super::choose_styled_candidate(&candidates, 0, evaluation),
            1
        );
        assert_eq!(
            super::selection_interest(&candidates[0], evaluation, 0),
            1_000,
        );
        assert!(super::selection_interest(&candidates[0], evaluation, 200) < 1_000);
    }

    #[test]
    fn root_line_outcome_recognizes_rule_fifty_and_repetition_draws() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 99 1").unwrap();
        let chess_move = find_move(&position, "h5g5");
        let history = super::RepetitionTracker::new(position.hash_history());

        assert_eq!(
            super::root_line_outcome(position.board(), &history, &[chess_move], 0, false),
            super::RootLineOutcome::ImmediateDraw,
        );
        assert_eq!(
            super::root_line_outcome(position.board(), &history, &[], 0, true),
            super::RootLineOutcome::RepetitionDraw,
        );
    }

    #[test]
    fn equal_major_exchange_is_sterile_but_a_clean_win_is_not() {
        let position = Position::from_fen("4k3/3q4/8/8/8/8/3Q4/4K3 w - - 0 1").unwrap();
        let trade: Move = "d2d7".parse().unwrap();
        let recapture: Move = "e8d7".parse().unwrap();
        assert!(position.board().is_legal(trade));
        let mut child = position.board().clone();
        child.play_unchecked(trade);
        assert!(child.is_legal(recapture));

        assert!(super::sterile_simplification(
            position.board(),
            &[trade, recapture],
            position.board().side_to_move(),
            0,
        ));
        assert!(!super::sterile_simplification(
            position.board(),
            &[trade],
            position.board().side_to_move(),
            0,
        ));
    }

    #[test]
    fn quiet_checks_are_recognized_without_being_tactical_captures() {
        let position = Position::from_fen("7k/8/8/8/8/8/4Q3/4K3 w - - 0 1").unwrap();
        let quiet_check = find_move(&position, "e2e8");
        let metadata = super::MoveMetadata::classify(position.board(), quiet_check);

        assert!(metadata.is_quiet());
        assert!(metadata.gives_check);
        assert!(!metadata.is_tactical());
    }
}
