use std::thread;
use std::time::{Duration, Instant};

use cozy_chess::util::display_uci_move;
use cozy_chess::{BitBoard, Board, GameStatus, Move, Piece};

use super::time::allocate_time;
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
struct NodeResult {
    score: Score,
    pv: Vec<Move>,
}

struct SearchContext<'a> {
    control: &'a SearchControl,
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
    mut report: F,
) -> SearchResult
where
    F: FnMut(SearchInfo),
{
    if let Some(duration) = allocate_time(position.board().side_to_move(), limits) {
        control.set_deadline_from_now(duration);
    }

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
        node_limit: limits.nodes,
        nodes: 0,
        started: Instant::now(),
    };
    let mut history = position.hash_history().to_vec();
    if !context.should_stop() && terminal_score(&root_board, &history, 0) == Some(0) {
        let info = SearchInfo::new(
            0,
            SearchScore::Centipawns(0),
            0,
            context.started.elapsed(),
            vec![fallback.clone().expect("root moves are non-empty")],
        );
        report(info.clone());
        wait_for_unbounded(limits, &context);
        return SearchResult::from_parts(fallback, Some(info));
    }
    let mut previous_pv = Vec::new();
    let mut final_info = None;
    let maximum_depth = maximum_depth(limits);

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

    wait_for_unbounded(limits, &context);
    let best_move = final_info
        .as_ref()
        .and_then(|info| info.pv().first().cloned())
        .or(fallback);
    SearchResult::from_parts(best_move, final_info)
}

fn wait_for_unbounded(limits: &SearchLimits, context: &SearchContext<'_>) {
    if (limits.infinite || limits.ponder) && limits.nodes.is_none() {
        while !context.should_stop() {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn maximum_depth(limits: &SearchLimits) -> u32 {
    let depth_limit = limits.depth.map(|depth| depth.clamp(1, MAX_DEPTH));
    let mate_limit = limits
        .mate
        .map(|moves| moves.saturating_mul(2).clamp(1, MAX_DEPTH));
    match (depth_limit, mate_limit) {
        (Some(depth), Some(mate)) => depth.min(mate),
        (Some(depth), None) => depth,
        (None, Some(mate)) => mate,
        (None, None)
            if limits.nodes.is_some()
                || limits.move_time.is_some()
                || limits.white_time.is_some()
                || limits.black_time.is_some()
                || limits.infinite
                || limits.ponder =>
        {
            MAX_DEPTH
        }
        (None, None) => DEFAULT_DEPTH,
    }
}

fn search_root(
    board: &Board,
    root_moves: &[Move],
    history: &mut Vec<u64>,
    depth: u32,
    previous_pv: &[Move],
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    let preferred = previous_pv.first().copied();
    let moves = order_moves(board, root_moves.to_vec(), preferred);
    let mut alpha = NEG_INFINITY;
    let beta = POS_INFINITY;
    let mut best = NodeResult {
        score: NEG_INFINITY,
        pv: Vec::new(),
    };

    for chess_move in moves {
        if context.should_stop() {
            return Err(Aborted);
        }

        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(repetition_key(&child));
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
            best = NodeResult { score, pv };
        }
        alpha = alpha.max(score);
    }

    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn negamax(
    board: &Board,
    history: &mut Vec<u64>,
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
    if let Some(score) = terminal_score(board, history, ply) {
        return Ok(NodeResult {
            score,
            pv: Vec::new(),
        });
    }
    if ply >= MAX_PLY {
        return Ok(NodeResult {
            score: evaluate(board),
            pv: Vec::new(),
        });
    }

    let preferred = previous_pv.first().copied();
    let moves = order_moves(board, generate_moves(board), preferred);
    let mut best = NodeResult {
        score: NEG_INFINITY,
        pv: Vec::new(),
    };

    for chess_move in moves {
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(repetition_key(&child));
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
            best = NodeResult { score, pv };
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }

    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn quiescence(
    board: &Board,
    history: &mut Vec<u64>,
    ply: u32,
    mut alpha: Score,
    beta: Score,
    remaining: u32,
    context: &mut SearchContext<'_>,
) -> Result<NodeResult, Aborted> {
    context.visit_node()?;
    if let Some(score) = terminal_score(board, history, ply) {
        return Ok(NodeResult {
            score,
            pv: Vec::new(),
        });
    }

    let in_check = !board.checkers().is_empty();
    let stand_pat = evaluate(board);
    if (remaining == 0 && !in_check) || ply >= MAX_PLY {
        return Ok(NodeResult {
            score: stand_pat,
            pv: Vec::new(),
        });
    }

    let mut best = NodeResult {
        score: if in_check { NEG_INFINITY } else { stand_pat },
        pv: Vec::new(),
    };
    if !in_check {
        if stand_pat >= beta {
            return Ok(best);
        }
        alpha = alpha.max(stand_pat);
    }

    let mut moves = generate_moves(board);
    if !in_check {
        moves.retain(|&chess_move| is_tactical(board, chess_move));
    }
    moves = order_moves(board, moves, None);

    for chess_move in moves {
        let mut child = board.clone();
        child.play_unchecked(chess_move);
        history.push(repetition_key(&child));
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
            best = NodeResult { score, pv };
        }
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }

    Ok(best)
}

fn terminal_score(board: &Board, history: &[u64], ply: u32) -> Option<Score> {
    let current = repetition_key(board);
    if history.iter().filter(|&&hash| hash == current).count() >= 3 || is_dead_material(board) {
        return Some(0);
    }

    match board.status() {
        GameStatus::Won => Some(-MATE_SCORE + ply as Score),
        GameStatus::Drawn => Some(0),
        GameStatus::Ongoing => None,
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
