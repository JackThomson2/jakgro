use std::collections::HashMap;
use std::time::{Duration, Instant};

use cozy_chess::util::display_uci_move;
use cozy_chess::{BitBoard, Board, Color, Move, Piece};

use super::control::DeadlineWindow;
use super::time::allocate_time;
use super::transposition::{Bound, TranspositionTable};
use super::{SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore};
use crate::engine::Position;
use crate::engine::evaluation::{
    EvaluationConfig, MATE_SCORE, MATE_THRESHOLD, MAX_PLY, NEG_INFINITY, POS_INFINITY, Score,
    evaluate_with_config, piece_value, root_complexity_bonus,
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

#[derive(Debug)]
struct RepetitionTracker {
    keys: Vec<u64>,
    counts: HashMap<u64, usize>,
}

impl RepetitionTracker {
    fn new(history: &[u64]) -> Self {
        let mut counts = HashMap::new();
        for &key in history {
            *counts.entry(key).or_insert(0) += 1;
        }

        Self {
            keys: history.to_vec(),
            counts,
        }
    }

    fn push(&mut self, board: &Board) {
        let key = repetition_key(board);
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

    fn occurrences(&self, board: &Board) -> usize {
        self.counts
            .get(&repetition_key(board))
            .copied()
            .unwrap_or(0)
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
struct RootCandidate {
    chess_move: Move,
    score: Score,
    path_dependent: bool,
    interest: i64,
    pv: Vec<Move>,
}

const HISTORY_MAX: u32 = 900_000;

#[derive(Debug)]
struct MoveOrdering {
    killers: Vec<[Option<Move>; 2]>,
    history: Vec<u32>,
}

impl MoveOrdering {
    fn new() -> Self {
        Self {
            killers: vec![[None; 2]; MAX_PLY as usize + 1],
            history: vec![0; 2 * 64 * 64],
        }
    }

    fn killers(&self, ply: u32) -> [Option<Move>; 2] {
        self.killers.get(ply as usize).copied().unwrap_or([None; 2])
    }

    fn history_score(&self, color: Color, chess_move: Move) -> u32 {
        self.history[history_index(color, chess_move)]
    }

    fn record_quiet_cutoff(&mut self, color: Color, chess_move: Move, ply: u32, depth: u32) {
        let killers = &mut self.killers[ply.min(MAX_PLY) as usize];
        if killers[0] != Some(chess_move) {
            killers[1] = killers[0];
            killers[0] = Some(chess_move);
        }

        let bonus = depth.saturating_mul(depth).min(HISTORY_MAX / 4);
        let index = history_index(color, chess_move);
        if self.history[index] > HISTORY_MAX - bonus {
            for score in &mut self.history {
                *score /= 2;
            }
        }
        self.history[index] = self.history[index].saturating_add(bonus).min(HISTORY_MAX);
    }
}

fn history_index(color: Color, chess_move: Move) -> usize {
    ((color as usize * 64 + chess_move.from as usize) * 64) + chess_move.to as usize
}
struct SearchContext<'a> {
    control: &'a SearchControl,
    table: &'a mut TranspositionTable,
    evaluation: EvaluationConfig,
    node_limit: Option<u64>,
    nodes: u64,
    started: Instant,
    pv: Vec<Vec<Move>>,
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
        self.pv[ply.min(MAX_PLY) as usize].clear();
    }

    fn update_pv(&mut self, ply: u32, chess_move: Move) {
        let ply = ply.min(MAX_PLY) as usize;
        let (current_rows, child_rows) = self.pv.split_at_mut(ply + 1);
        let current = &mut current_rows[ply];
        current.clear();
        current.push(chess_move);
        if let Some(child) = child_rows.first() {
            current.extend_from_slice(child);
        }
    }

    fn write_hash_pv(&mut self, board: &Board, depth: u32, ply: u32) {
        let output = &mut self.pv[ply.min(MAX_PLY) as usize];
        self.table.write_principal_variation(board, depth, output);
    }

    fn pv(&self, ply: u32) -> &[Move] {
        &self.pv[ply.min(MAX_PLY) as usize]
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
        return SearchResult::from_parts(None, None);
    }
    let root_moves = labeled_moves
        .into_iter()
        .map(|(_, chess_move)| chess_move)
        .collect::<Vec<_>>();

    let mut context = SearchContext {
        control,
        table,
        evaluation,
        node_limit: limits.nodes,
        nodes: 0,
        started: Instant::now(),
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
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
        return SearchResult::from_parts(fallback, Some(info));
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
        let iteration = loop {
            let iteration = search_root(
                &root_board,
                &root_moves,
                &mut history,
                depth,
                (alpha, beta),
                &previous_pv,
                &mut context,
            );
            let Ok(iteration) = iteration else {
                break 'iterative;
            };

            if iteration.score > alpha && iteration.score < beta {
                break iteration;
            }
            if alpha == NEG_INFINITY && beta == POS_INFINITY {
                break iteration;
            }

            radius = radius.saturating_mul(2);
            if iteration.score.abs() >= MATE_THRESHOLD || radius >= POS_INFINITY {
                alpha = NEG_INFINITY;
                beta = POS_INFINITY;
            } else if let Some(score) = previous_score {
                (alpha, beta) = aspiration_bounds(score, radius);
            }
        };
        let iteration_duration = iteration_started.elapsed();

        let is_volatile = stability.observe(context.pv(0).first().copied(), iteration.score);
        previous_score = Some(iteration.score);
        previous_pv.clear();
        previous_pv.extend_from_slice(context.pv(0));
        let pv = format_pv(&root_board, &previous_pv);
        let info = SearchInfo::new(
            depth,
            SearchScore::from_internal(iteration.score),
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
    SearchResult::from_parts(best_move, final_info)
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
) -> Result<NodeResult, Aborted> {
    if context.evaluation.root_style_margin() == 0 {
        return search_root_conventional(
            board,
            root_moves,
            history,
            depth,
            window,
            previous_pv,
            context,
        );
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
) -> Result<NodeResult, Aborted> {
    let objective = search_root_conventional(
        board,
        root_moves,
        history,
        depth,
        window,
        previous_pv,
        context,
    )?;
    let objective_pv = context.pv(0).to_vec();
    let Some(objective_move) = objective_pv.first().copied() else {
        return Ok(objective);
    };
    if objective.score.abs() >= MATE_THRESHOLD
        || objective.score <= window.0
        || objective.score >= window.1
        || context.should_stop()
    {
        return Ok(objective);
    }

    let mut objective_child = board.clone();
    objective_child.play_unchecked(objective_move);
    let mut candidates = vec![RootCandidate {
        chess_move: objective_move,
        score: objective.score,
        path_dependent: objective.path_dependent,
        interest: root_interest(board, &objective_child, objective_move, context.evaluation),
        pv: objective_pv,
    }];
    let mut alternatives = root_moves
        .iter()
        .copied()
        .filter(|&chess_move| chess_move != objective_move)
        .map(|chess_move| {
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            let interest = root_interest(board, &child, chess_move, context.evaluation);
            (chess_move, interest)
        })
        .collect::<Vec<_>>();
    alternatives.sort_unstable_by(|(left_move, left_interest), (right_move, right_interest)| {
        right_interest
            .cmp(left_interest)
            .then_with(|| move_key(*left_move).cmp(&move_key(*right_move)))
    });
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        !board.checkers().is_empty(),
        0,
        context.evaluation.max_check_extensions(),
    );

    for (chess_move, interest) in alternatives.into_iter().take(6) {
        if context.should_stop() {
            break;
        }
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(&child);
        let child_result = negamax(
            &child,
            history,
            child_depth,
            1,
            child_extensions,
            NEG_INFINITY,
            POS_INFINITY,
            &[],
            context,
        );
        history.pop();
        let Ok(child_result) = child_result else {
            break;
        };
        let mut pv = vec![chess_move];
        pv.extend_from_slice(context.pv(1));
        candidates.push(RootCandidate {
            chess_move,
            score: -child_result.score,
            path_dependent: child_result.path_dependent,
            interest,
            pv,
        });
    }

    let selected = choose_styled_candidate(&candidates, 0, context.evaluation.root_style_margin());
    context.pv[0].clear();
    context.pv[0].extend_from_slice(&candidates[selected].pv);
    Ok(NodeResult {
        score: candidates[selected].score,
        path_dependent: candidates[selected].path_dependent,
    })
}

fn choose_styled_candidate(
    candidates: &[RootCandidate],
    conventional: usize,
    margin: Score,
) -> usize {
    let best = candidates[conventional].score;
    if best.abs() >= MATE_THRESHOLD {
        return conventional;
    }
    let mut selected = conventional;
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.score.abs() >= MATE_THRESHOLD || candidate.score < best - margin {
            continue;
        }
        let current = &candidates[selected];
        if candidate.interest > current.interest
            || (candidate.interest == current.interest && candidate.score > current.score)
            || (candidate.interest == current.interest
                && candidate.score == current.score
                && move_key(candidate.chess_move) < move_key(current.chess_move))
        {
            selected = index;
        }
    }
    selected
}

fn search_root_conventional(
    board: &Board,
    root_moves: &[Move],
    history: &mut RepetitionTracker,
    depth: u32,
    window: (Score, Score),
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    let (mut alpha, beta) = window;
    context.clear_pv(0);
    if context.should_stop() {
        return Err(Aborted);
    }
    let alpha_original = alpha;
    let hash_move = context
        .table
        .probe(board)
        .and_then(|entry| entry.best_move());
    let preferred = previous_pv.first().copied().or(hash_move);
    let moves = order_root_moves(
        board,
        root_moves.to_vec(),
        preferred,
        &context.ordering,
        context.evaluation,
    );
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        !board.checkers().is_empty(),
        0,
        context.evaluation.max_check_extensions(),
    );
    let mut best = NodeResult {
        score: NEG_INFINITY,
        path_dependent: false,
    };

    for (index, chess_move) in moves.into_iter().enumerate() {
        if context.should_stop() {
            return Err(Aborted);
        }

        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        history.push(&child);
        let first_window = if index == 0 {
            (-beta, -alpha)
        } else {
            (-alpha - 1, -alpha)
        };
        let mut child_result = negamax(
            &child,
            history,
            child_depth,
            1,
            child_extensions,
            first_window.0,
            first_window.1,
            expected_child_pv,
            context,
        );
        history.pop();
        let mut score = -child_result.as_ref().map_err(|_| Aborted)?.score;

        if index != 0 && score > alpha && score < beta {
            history.push(&child);
            child_result = negamax(
                &child,
                history,
                child_depth,
                1,
                child_extensions,
                -beta,
                -alpha,
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
            context.update_pv(0, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }

    if !best.path_dependent {
        let bound = if best.score <= alpha_original {
            Bound::Upper
        } else if best.score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        context.table.store(
            board,
            depth,
            0,
            best.score,
            bound,
            context.pv(0).first().copied(),
        );
    }
    Ok(best)
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
            context.evaluation.quiescence_check_budget(),
            context,
        );
    }

    context.clear_pv(ply);
    context.visit_node()?;
    let moves = generate_moves(board);
    if let Some(result) = terminal_score(board, history, ply, moves.is_empty()) {
        return Ok(NodeResult {
            score: result.score,
            path_dependent: result.path_dependent,
        });
    }
    if ply >= MAX_PLY {
        return Ok(NodeResult {
            score: evaluate_with_config(board, context.evaluation),
            path_dependent: false,
        });
    }

    let (mate_alpha, mate_beta) = mate_distance_bounds(ply);
    alpha = alpha.max(mate_alpha);
    beta = beta.min(mate_beta);
    if alpha >= beta {
        return Ok(NodeResult {
            score: alpha,
            path_dependent: false,
        });
    }

    let alpha_original = alpha;
    let hash_entry = context.table.probe(board);
    if let Some(entry) = hash_entry.filter(|entry| entry.depth() >= depth) {
        let score = entry.score_at_ply(ply);
        let cutoff = match entry.bound() {
            Bound::Exact => true,
            Bound::Lower => score >= beta,
            Bound::Upper => score <= alpha,
        };
        if cutoff {
            context.write_hash_pv(board, depth, ply);
            return Ok(NodeResult {
                score,
                path_dependent: false,
            });
        }
    }

    let hash_move = hash_entry.and_then(|entry| entry.best_move());
    let preferred = previous_pv.first().copied().or(hash_move);
    let moves = order_moves_with_evaluation(
        board,
        moves,
        preferred,
        ply,
        &context.ordering,
        context.evaluation,
    );
    let (child_depth, child_extensions) = next_search_depth(
        depth,
        !board.checkers().is_empty(),
        extensions_used,
        context.evaluation.max_check_extensions(),
    );
    let mut best = NodeResult {
        score: NEG_INFINITY,
        path_dependent: false,
    };

    for (index, chess_move) in moves.into_iter().enumerate() {
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        history.push(&child);
        let first_window = if index == 0 {
            (-beta, -alpha)
        } else {
            (-alpha - 1, -alpha)
        };
        let mut child_result = negamax(
            &child,
            history,
            child_depth,
            ply + 1,
            child_extensions,
            first_window.0,
            first_window.1,
            expected_child_pv,
            context,
        );
        history.pop();
        let mut score = -child_result.as_ref().map_err(|_| Aborted)?.score;

        if index != 0 && score > alpha && score < beta {
            history.push(&child);
            child_result = negamax(
                &child,
                history,
                child_depth,
                ply + 1,
                child_extensions,
                -beta,
                -alpha,
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
            context.update_pv(ply, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            if is_quiet(board, chess_move) {
                context
                    .ordering
                    .record_quiet_cutoff(board.side_to_move(), chess_move, ply, depth);
            }
            break;
        }
    }

    if !best.path_dependent {
        let bound = if best.score <= alpha_original {
            Bound::Upper
        } else if best.score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        context.table.store(
            board,
            depth,
            ply,
            best.score,
            bound,
            context.pv(ply).first().copied(),
        );
    }

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
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    context.clear_pv(ply);
    context.visit_node()?;
    let mut moves = generate_moves(board);
    if let Some(result) = terminal_score(board, history, ply, moves.is_empty()) {
        return Ok(NodeResult {
            score: result.score,
            path_dependent: result.path_dependent,
        });
    }

    let (mate_alpha, mate_beta) = mate_distance_bounds(ply);
    alpha = alpha.max(mate_alpha);
    beta = beta.min(mate_beta);
    if alpha >= beta {
        return Ok(NodeResult {
            score: alpha,
            path_dependent: false,
        });
    }

    let in_check = !board.checkers().is_empty();
    let stand_pat = evaluate_with_config(board, context.evaluation);
    if (remaining == 0 && !in_check) || ply >= MAX_PLY {
        return Ok(NodeResult {
            score: stand_pat,
            path_dependent: false,
        });
    }

    let mut best = NodeResult {
        score: if in_check { NEG_INFINITY } else { stand_pat },
        path_dependent: false,
    };
    if !in_check {
        if stand_pat >= beta {
            return Ok(best);
        }
        alpha = alpha.max(stand_pat);
        moves.retain(|&chess_move| {
            is_tactical(board, chess_move)
                || (check_budget > 0
                    && is_quiet(board, chess_move)
                    && gives_check(board, chess_move))
        });
    }
    moves = order_moves_with_evaluation(
        board,
        moves,
        None,
        ply,
        &context.ordering,
        context.evaluation,
    );

    for chess_move in moves {
        let uses_quiet_check = !in_check
            && check_budget > 0
            && is_quiet(board, chess_move)
            && gives_check(board, chess_move);
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(&child);
        let child_result = quiescence(
            &child,
            history,
            ply + 1,
            -beta,
            -alpha,
            remaining.saturating_sub(1),
            check_budget.saturating_sub(u8::from(uses_quiet_check)),
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
            context.update_pv(ply, chess_move);
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }

    Ok(best)
}

fn terminal_score(
    board: &Board,
    history: &RepetitionTracker,
    ply: u32,
    no_legal_moves: bool,
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

    if history.occurrences(board) >= 3 {
        return Some(TerminalResult {
            score: 0,
            path_dependent: true,
        });
    }
    if board.halfmove_clock() >= 100 || is_dead_material(board) {
        return Some(TerminalResult {
            score: 0,
            path_dependent: false,
        });
    }

    None
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

fn generate_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

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

fn order_moves_with_evaluation(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ply: u32,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<Move> {
    let mut ranked = moves
        .into_iter()
        .map(|chess_move| {
            let score = move_order_score(board, chess_move, preferred, ply, ordering, evaluation);
            (chess_move, score)
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left, left_score), (right, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| move_key(*left).cmp(&move_key(*right)))
    });
    ranked
        .into_iter()
        .map(|(chess_move, _)| chess_move)
        .collect()
}
fn order_root_moves(
    board: &Board,
    moves: Vec<Move>,
    preferred: Option<Move>,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> Vec<Move> {
    if evaluation.aggression() == 0 {
        return order_moves(board, moves, preferred, 0, ordering);
    }
    let mover = board.side_to_move();
    let mut ranked = moves
        .into_iter()
        .map(|chess_move| {
            let mut child = board.clone();
            child.play_unchecked(chess_move);
            let complexity = root_complexity_bonus(&child, mover, evaluation);
            (chess_move, complexity)
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left, left_complexity), (right, right_complexity)| {
        move_order_score(board, *right, preferred, 0, ordering, evaluation)
            .cmp(&move_order_score(
                board, *left, preferred, 0, ordering, evaluation,
            ))
            .then_with(|| right_complexity.cmp(left_complexity))
            .then_with(|| move_key(*left).cmp(&move_key(*right)))
    });
    ranked
        .into_iter()
        .map(|(chess_move, _)| chess_move)
        .collect()
}

fn move_order_score(
    board: &Board,
    chess_move: Move,
    preferred: Option<Move>,
    ply: u32,
    ordering: &MoveOrdering,
    evaluation: EvaluationConfig,
) -> i64 {
    if preferred == Some(chess_move) {
        return 6_000_000;
    }

    if let Some(promotion) = chess_move.promotion {
        return 5_000_000 + i64::from(piece_value(promotion)) * 32;
    }

    if let Some(captured) = captured_piece(board, chess_move) {
        let attacker = board.piece_on(chess_move.from).unwrap_or(Piece::King);
        let captured_value = ordering_piece_value(captured);
        let attacker_value = ordering_piece_value(attacker);
        let exchange = i64::from(captured_value) * 32 - i64::from(attacker_value);
        return if captured_value >= attacker_value {
            4_000_000 + exchange
        } else {
            1_000_000 + exchange
        };
    }

    let history = i64::from(ordering.history_score(board.side_to_move(), chess_move));
    let forcing = forcing_order_bonus(board, chess_move, evaluation);
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
    chess_move: Move,
    evaluation: EvaluationConfig,
) -> i64 {
    let mover = board.side_to_move();
    let mut interest = i64::from(root_complexity_bonus(child, mover, evaluation)) * 10;
    interest += i64::from(gives_check(board, chess_move)) * 120;
    interest += i64::from(is_attacking_pawn_push(board, chess_move)) * 40;
    interest += chess_move
        .promotion
        .map_or(0, |piece| i64::from(piece_value(piece)) / 5);
    interest += i64::from(child.pieces(Piece::Queen).len()) * 20;
    interest += i64::from(total_non_pawn_material(child)) / 100;
    interest += i64::from(material_balance(child).abs()) / 25;
    interest += i64::from(captured_piece(board, chess_move).is_none()) * 15;
    interest
}

fn total_non_pawn_material(board: &Board) -> Score {
    [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen]
        .into_iter()
        .map(|piece| piece_value(piece) * board.pieces(piece).len() as Score)
        .sum()
}

fn material_balance(board: &Board) -> Score {
    [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
    ]
    .into_iter()
    .map(|piece| {
        piece_value(piece)
            * (board.colored_pieces(Color::White, piece).len() as Score
                - board.colored_pieces(Color::Black, piece).len() as Score)
    })
    .sum()
}

fn forcing_order_bonus(board: &Board, chess_move: Move, evaluation: EvaluationConfig) -> i64 {
    let aggression = i64::from(evaluation.aggression());
    if aggression == 0 {
        return 0;
    }
    if gives_check(board, chess_move) {
        return 800_000 + aggression * 3_000;
    }
    if is_attacking_pawn_push(board, chess_move) {
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

fn is_quiet(board: &Board, chess_move: Move) -> bool {
    chess_move.promotion.is_none() && captured_piece(board, chess_move).is_none()
}
fn gives_check(board: &Board, chess_move: Move) -> bool {
    let mut child = board.clone();
    child.play_unchecked(chess_move);
    !child.checkers().is_empty()
}
fn is_tactical(board: &Board, chess_move: Move) -> bool {
    chess_move.promotion.is_some() || captured_piece(board, chess_move).is_some()
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
        MATE_SCORE, MoveOrdering, RepetitionTracker, generate_moves, order_moves, terminal_score,
    };
    use crate::engine::Position;

    #[test]
    fn repetition_tracker_pushes_and_pops_in_constant_time() {
        let position = Position::default();
        let mut tracker = RepetitionTracker::new(position.hash_history());

        assert_eq!(tracker.occurrences(position.board()), 1);
        tracker.push(position.board());
        assert_eq!(tracker.occurrences(position.board()), 2);
        tracker.pop();
        assert_eq!(tracker.occurrences(position.board()), 1);
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
    fn root_complexity_orders_equally_ranked_quiet_moves() {
        let position = Position::from_fen("6k1/5ppp/8/7Q/2B5/8/5PPP/6K1 w - - 0 1").unwrap();
        let quiet_moves = generate_moves(position.board())
            .into_iter()
            .filter(|&chess_move| super::is_quiet(position.board(), chess_move))
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
    fn quiet_cutoffs_install_bounded_killer_and_history_scores() {
        let position = Position::default();
        let chess_move = find_move(&position, "d2d4");
        let mut ordering = MoveOrdering::new();

        ordering.record_quiet_cutoff(position.board().side_to_move(), chess_move, 3, 8);
        let ordered = order_moves(
            position.board(),
            position.search_moves(),
            None,
            3,
            &ordering,
        );

        assert_eq!(ordered[0], chess_move);
        assert!(
            ordering.history_score(position.board().side_to_move(), chess_move)
                <= super::HISTORY_MAX
        );
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

        assert!(super::gives_check(position.board(), ordered[0]));
        assert!(super::is_quiet(position.board(), ordered[0]));
    }

    #[test]
    fn root_style_margin_scales_from_zero_to_a_pawn() {
        assert_eq!(super::EvaluationConfig::new(0).root_style_margin(), 0);
        assert_eq!(super::EvaluationConfig::new(50).root_style_margin(), 30);
        assert_eq!(super::EvaluationConfig::new(100).root_style_margin(), 120);
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
            },
            super::RootCandidate {
                chess_move: exciting_move,
                score: -60,
                path_dependent: false,
                interest: 100,
                pv: vec![exciting_move],
            },
        ];

        assert_eq!(super::choose_styled_candidate(&candidates, 0, 120), 1);
        candidates[1].score = -71;
        assert_eq!(super::choose_styled_candidate(&candidates, 0, 120), 0);
        candidates[0].score = super::MATE_SCORE - 1;
        candidates[1].score = super::MATE_SCORE - 2;
        assert_eq!(super::choose_styled_candidate(&candidates, 0, 120), 0);
    }

    #[test]
    fn quiet_checks_are_recognized_without_being_tactical_captures() {
        let position = Position::from_fen("7k/8/8/8/8/8/4Q3/4K3 w - - 0 1").unwrap();
        let quiet_check = find_move(&position, "e2e8");

        assert!(super::is_quiet(position.board(), quiet_check));
        assert!(super::gives_check(position.board(), quiet_check));
        assert!(!super::is_tactical(position.board(), quiet_check));
    }
}
