use cozy_chess::Board;

#[cfg(test)]
mod tests;

use crate::engine::evaluation::{
    MAX_PLY,
    nnue::{Accumulator, Network},
};

/// One worker's lazy accumulator rows. Every row remembers its actual piece
/// placement; skipped parents and same-ply re-searches need no push/pop hooks.
pub(super) struct NeuralEvaluator<'network> {
    network: &'network Network,
    rows: Box<[Option<Accumulator<'network>>]>,
}

impl<'network> NeuralEvaluator<'network> {
    pub(super) fn new(network: &'network Network) -> Self {
        Self {
            network,
            rows: (0..=MAX_PLY).map(|_| None).collect(),
        }
    }

    pub(super) fn evaluate(&mut self, board: &Board, ply: u32) -> i32 {
        let row = &mut self.rows[ply.min(MAX_PLY) as usize];
        match row {
            Some(accumulator) => accumulator.update(board),
            None => *row = Some(self.network.accumulator(board)),
        }
        row.as_ref()
            .expect("the accumulator row is initialized")
            .evaluate()
    }
}
