use cozy_chess::Board;

#[cfg(test)]
mod tests;

use crate::engine::evaluation::{
    MAX_PLY,
    nnue::{AccumulatorStack, Network},
};

/// One worker's evaluation states, one per ply. Every state remembers its
/// actual piece placement; skipped parents and same-ply re-searches need no
/// push/pop hooks.
pub(super) struct NeuralEvaluator<'network> {
    stack: AccumulatorStack<'network>,
}

impl<'network> NeuralEvaluator<'network> {
    pub(super) fn new(network: &'network Network) -> Self {
        Self {
            stack: AccumulatorStack::new(network, MAX_PLY as usize),
        }
    }

    pub(super) fn evaluate(&mut self, board: &Board, ply: u32) -> i32 {
        self.stack.evaluate(board, ply.min(MAX_PLY) as usize)
    }
}
