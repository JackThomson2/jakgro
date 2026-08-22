use std::collections::HashMap;
use std::time::Instant;

use cozy_chess::util::display_uci_move;
use cozy_chess::{BitBoard, Board, Move, Piece};

use super::time::allocate_time;
use super::transposition::{Bound, TranspositionTable};
use super::{SearchControl, SearchInfo, SearchLimits, SearchResult, SearchScore};
use crate::engine::Position;
use crate::engine::evaluation::{
    MATE_SCORE, MAX_PLY, NEG_INFINITY, POS_INFINITY, Score, evaluate, piece_value,
};
use crate::engine::position::repetition_key;

const DEFAULT_DEPTH: u32 = 4;
const MAX_DEPTH: u32 = 64;
const QUIESCENCE_DEPTH: u32 = 16;

#[derive(Debug)]
struct Aborted;

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
    pv: Vec<Move>,
    path_dependent: bool,
}

struct SearchContext<'a> {
    control: &'a SearchControl,
    table: &'a mut TranspositionTable,
    node_limit: Option<u64>,
    nodes: u64,
    started: Instant,
}

impl SearchContext<'_> {
    fn visit_node(&mut self) -> Result<(), Aborted> {
        if self.control.is_stopped()
            || self
                .node_limit
                .is_some_and(|node_limit| self.nodes >= node_limit)
        {
            return Err(Aborted);
        }

        self.nodes += 1;
        if self.control.deadline_reached() {
            return Err(Aborted);
        }
        Ok(())
    }

    fn should_stop(&self) -> bool {
        self.control.is_stopped()
            || self.control.deadline_reached()
            || self
                .node_limit
                .is_some_and(|node_limit| self.nodes >= node_limit)
    }
}

pub(super) fn run<F>(
    position: &Position,
    limits: &SearchLimits,
    control: &SearchControl,
    table: &mut TranspositionTable,
    mut report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    let time_budget = allocate_time(position.board().side_to_move(), limits);
    if let Some(duration) = time_budget {
        control.set_deadline_from_now(duration);
    }

    table.start_search();
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
        node_limit: limits.nodes,
        nodes: 0,
        started: Instant::now(),
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
    let mut final_info = None;
    let maximum_depth = maximum_depth(limits, time_budget.is_some());

    for depth in 1..=maximum_depth {
        if context.should_stop() {
            break;
        }

        let iteration = search_root(
            &root_board,
            &root_moves,
            &mut history,
            depth,
            &previous_pv,
            &mut context,
        );
        let Ok(iteration) = iteration else {
            break;
        };

        previous_pv = iteration.pv;
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

fn search_root(
    board: &Board,
    root_moves: &[Move],
    history: &mut RepetitionTracker,
    depth: u32,
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    let hash_move = context
        .table
        .probe(board)
        .and_then(|entry| entry.best_move());
    let preferred = previous_pv.first().copied().or(hash_move);
    let moves = order_moves(board, root_moves.to_vec(), preferred);
    let mut alpha = NEG_INFINITY;
    let beta = POS_INFINITY;
    let mut best = NodeResult {
        score: NEG_INFINITY,
        pv: Vec::new(),
        path_dependent: false,
    };

    for chess_move in moves {
        if context.should_stop() {
            return Err(Aborted);
        }

        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(&child);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        let child_result = negamax(
            &child,
            history,
            depth - 1,
            1,
            -beta,
            -alpha,
            expected_child_pv,
            context,
        );
        history.pop();
        let child_result = child_result?;
        let score = -child_result.score;

        if score > best.score {
            let mut pv = Vec::with_capacity(child_result.pv.len() + 1);
            pv.push(chess_move);
            pv.extend(child_result.pv);
            best = NodeResult {
                score,
                pv,
                path_dependent: child_result.path_dependent,
            };
        }
        alpha = alpha.max(score);
    }

    if !best.path_dependent {
        context.table.store(
            board,
            depth,
            0,
            best.score,
            Bound::Exact,
            best.pv.first().copied(),
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
    mut alpha: Score,
    beta: Score,
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    if depth == 0 {
        return quiescence(board, history, ply, alpha, beta, QUIESCENCE_DEPTH, context);
    }

    context.visit_node()?;
    let moves = generate_moves(board);
    if let Some(result) = terminal_score(board, history, ply, moves.is_empty()) {
        return Ok(NodeResult {
            score: result.score,
            pv: Vec::new(),
            path_dependent: result.path_dependent,
        });
    }
    if ply >= MAX_PLY {
        return Ok(NodeResult {
            score: evaluate(board),
            pv: Vec::new(),
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
            return Ok(NodeResult {
                score,
                pv: context.table.principal_variation(board, depth),
                path_dependent: false,
            });
        }
    }

    let hash_move = hash_entry.and_then(|entry| entry.best_move());
    let preferred = previous_pv.first().copied().or(hash_move);
    let moves = order_moves(board, moves, preferred);
    let mut best = NodeResult {
        score: NEG_INFINITY,
        pv: Vec::new(),
        path_dependent: false,
    };

    for chess_move in moves {
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(&child);
        let expected_child_pv = if preferred == Some(chess_move) {
            previous_pv.get(1..).unwrap_or_default()
        } else {
            &[]
        };
        let child_result = negamax(
            &child,
            history,
            depth - 1,
            ply + 1,
            -beta,
            -alpha,
            expected_child_pv,
            context,
        );
        history.pop();
        let child_result = child_result?;
        let score = -child_result.score;

        if score > best.score {
            let mut pv = Vec::with_capacity(child_result.pv.len() + 1);
            pv.push(chess_move);
            pv.extend(child_result.pv);
            best = NodeResult {
                score,
                pv,
                path_dependent: child_result.path_dependent,
            };
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
            ply,
            best.score,
            bound,
            best.pv.first().copied(),
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
    beta: Score,
    remaining: u32,
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    context.visit_node()?;
    let mut moves = generate_moves(board);
    if let Some(result) = terminal_score(board, history, ply, moves.is_empty()) {
        return Ok(NodeResult {
            score: result.score,
            pv: Vec::new(),
            path_dependent: result.path_dependent,
        });
    }

    let in_check = !board.checkers().is_empty();
    let stand_pat = evaluate(board);
    if (remaining == 0 && !in_check) || ply >= MAX_PLY {
        return Ok(NodeResult {
            score: stand_pat,
            pv: Vec::new(),
            path_dependent: false,
        });
    }

    let mut best = NodeResult {
        score: if in_check { NEG_INFINITY } else { stand_pat },
        pv: Vec::new(),
        path_dependent: false,
    };
    if !in_check {
        if stand_pat >= beta {
            return Ok(best);
        }
        alpha = alpha.max(stand_pat);
        moves.retain(|&chess_move| is_tactical(board, chess_move));
    }
    moves = order_moves(board, moves, None);

    for chess_move in moves {
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
            context,
        );
        history.pop();
        let child_result = child_result?;
        let score = -child_result.score;

        if score > best.score {
            let mut pv = Vec::with_capacity(child_result.pv.len() + 1);
            pv.push(chess_move);
            pv.extend(child_result.pv);
            best = NodeResult {
                score,
                pv,
                path_dependent: child_result.path_dependent,
            };
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

fn order_moves(board: &Board, moves: Vec<Move>, preferred: Option<Move>) -> Vec<Move> {
    let mut scored = moves
        .into_iter()
        .map(|chess_move| {
            let mut priority = 0;
            if preferred == Some(chess_move) {
                priority += 1_000_000;
            }
            if let Some(promotion) = chess_move.promotion {
                priority += 100_000 + piece_value(promotion);
            }
            if let Some(captured) = captured_piece(board, chess_move) {
                let attacker = board.piece_on(chess_move.from).unwrap_or(Piece::Pawn);
                priority += 10_000 + piece_value(captured) * 10 - piece_value(attacker);
            }
            (
                chess_move,
                priority,
                display_uci_move(board, chess_move).to_string(),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.2.cmp(&right.2)));
    scored
        .into_iter()
        .map(|(chess_move, _, _)| chess_move)
        .collect()
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
    use super::{MATE_SCORE, RepetitionTracker, generate_moves, terminal_score};
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
}
