//! Syzygy endgame tablebases.
//!
//! The probing code is `pyrrhic-rs`, which keeps its tables in process-wide
//! state. Only one set can exist at a time and its root probe must not run
//! concurrently with itself, so [`Tablebases::load`] shares one set per path,
//! finishes freeing a set before it starts another, and serializes the root
//! probe. Searches only ever borrow the set.

use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use cozy_chess::{
    BitBoard, Board, Color, Move, Piece, Rank, Square, get_bishop_moves, get_king_moves,
    get_knight_moves, get_pawn_attacks, get_rook_moves,
};
use pyrrhic_rs::{DtzProbeValue, EngineAdapter, TBError, TableBases, WdlProbeResult};

use super::Engine;
use super::evaluation::{Score, TB_WIN_SCORE};

#[cfg(test)]
mod tests;

/// The `SyzygyPath` value that removes the tablebases.
pub const NO_SYZYGY_PATH: &str = "<empty>";
/// Fewest plies of remaining depth at which a position with the most pieces the
/// tables cover is probed.
pub const MIN_SYZYGY_PROBE_DEPTH: u32 = 1;
/// Default `SyzygyProbeDepth`.
pub const DEFAULT_SYZYGY_PROBE_DEPTH: u32 = 1;
/// Largest accepted `SyzygyProbeDepth`.
pub const MAX_SYZYGY_PROBE_DEPTH: u32 = 100;
/// Default `SyzygyProbeLimit`, which the tables' own size then narrows.
pub const DEFAULT_SYZYGY_PROBE_LIMIT: u32 = 7;
/// Largest accepted `SyzygyProbeLimit`.
pub const MAX_SYZYGY_PROBE_LIMIT: u32 = 7;

/// Rank of a root move that keeps a win inside the fifty-move rule.
const MAX_DTZ: i32 = 1 << 18;

/// Why a set of tablebases could not be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TablebaseError {
    /// The path names no directory.
    EmptyPath,
    /// No directory of the path holds a Syzygy table.
    NoTables,
    /// Another set of tablebases, loaded from a different path, is in use.
    InUse,
    /// The probing library rejected the tables.
    InitFailed,
}

impl Display for TablebaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyPath => "SyzygyPath requires a non-empty path",
            Self::NoTables => "no Syzygy tablebase files were found in the path",
            Self::InUse => "tablebases from another path are still in use",
            Self::InitFailed => "the tablebases could not be initialized",
        })
    }
}

impl Error for TablebaseError {}

/// What a load found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TablebaseSummary {
    /// Win-draw-loss files in the path.
    pub wdl_files: usize,
    /// Distance-to-zero files in the path.
    pub dtz_files: usize,
    /// Most pieces, kings included, that a position can have and be probed.
    pub largest: usize,
}

/// The theoretical result of a position for the side to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Wdl {
    Loss,
    /// Lost, but the fifty-move rule makes it a draw.
    BlessedLoss,
    Draw,
    /// Won, but the fifty-move rule makes it a draw.
    CursedWin,
    Win,
}

impl From<WdlProbeResult> for Wdl {
    fn from(result: WdlProbeResult) -> Self {
        match result {
            WdlProbeResult::Loss => Self::Loss,
            WdlProbeResult::BlessedLoss => Self::BlessedLoss,
            WdlProbeResult::Draw => Self::Draw,
            WdlProbeResult::CursedWin => Self::CursedWin,
            WdlProbeResult::Win => Self::Win,
        }
    }
}

impl Wdl {
    /// The same result seen from the other side.
    fn reversed(self) -> Self {
        match self {
            Self::Loss => Self::Win,
            Self::BlessedLoss => Self::CursedWin,
            Self::Draw => Self::Draw,
            Self::CursedWin => Self::BlessedLoss,
            Self::Win => Self::Loss,
        }
    }
}

/// The moves of a tablebase root position that keep its best result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RootRanking {
    /// The moves of the best rank, in the order they were given.
    pub(super) moves: Vec<Move>,
    /// What the best rank is worth to the side to move.
    pub(super) score: Score,
}

/// The engine's own attack generation, which the probing code borrows.
#[derive(Clone)]
struct Adapter;

fn square_at(index: u64) -> Square {
    Square::index(index as usize)
}

impl EngineAdapter for Adapter {
    fn pawn_attacks(color: pyrrhic_rs::Color, index: u64) -> u64 {
        let color = match color {
            pyrrhic_rs::Color::White => Color::White,
            pyrrhic_rs::Color::Black => Color::Black,
        };
        get_pawn_attacks(square_at(index), color).0
    }

    fn knight_attacks(index: u64) -> u64 {
        get_knight_moves(square_at(index)).0
    }

    fn bishop_attacks(index: u64, occupied: u64) -> u64 {
        get_bishop_moves(square_at(index), BitBoard(occupied)).0
    }

    fn rook_attacks(index: u64, occupied: u64) -> u64 {
        get_rook_moves(square_at(index), BitBoard(occupied)).0
    }

    fn queen_attacks(index: u64, occupied: u64) -> u64 {
        Self::bishop_attacks(index, occupied) | Self::rook_attacks(index, occupied)
    }

    fn king_attacks(index: u64) -> u64 {
        get_king_moves(square_at(index)).0
    }
}

/// A position in the layout the probing code takes.
struct Bits {
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    rooks: u64,
    bishops: u64,
    knights: u64,
    pawns: u64,
    en_passant: u32,
    white_to_move: bool,
}

impl Bits {
    fn of(board: &Board) -> Self {
        let side = board.side_to_move();
        Self {
            white: board.colors(Color::White).0,
            black: board.colors(Color::Black).0,
            kings: board.pieces(Piece::King).0,
            queens: board.pieces(Piece::Queen).0,
            rooks: board.pieces(Piece::Rook).0,
            bishops: board.pieces(Piece::Bishop).0,
            knights: board.pieces(Piece::Knight).0,
            pawns: board.pieces(Piece::Pawn).0,
            en_passant: board.en_passant().map_or(0, |file| {
                Square::new(file, Rank::Sixth.relative_to(side)) as u32
            }),
            white_to_move: side == Color::White,
        }
    }
}

/// Serializes loading and freeing, which the probing library keeps in
/// process-wide state.
static ACTIVE: Mutex<Weak<Tablebases>> = Mutex::new(Weak::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// A set of Syzygy tablebases.
pub(super) struct Tablebases {
    /// Empty only while the set is being dropped.
    probe: Option<TableBases<Adapter>>,
    path: String,
    summary: TablebaseSummary,
    /// The library's root probe is not thread safe.
    root_probe: Mutex<()>,
}

impl Debug for Tablebases {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tablebases")
            .field("path", &self.path)
            .field("summary", &self.summary)
            .finish()
    }
}

impl Drop for Tablebases {
    fn drop(&mut self) {
        // Freeing the tables and clearing the library's flag has to be over
        // before another load may look at it.
        let _active = lock(&ACTIVE);
        self.probe.take();
    }
}

impl Tablebases {
    /// Loads the tables of `path`, a list of directories separated by colons,
    /// or shares the set already loaded from it.
    pub(super) fn load(path: &str) -> Result<Arc<Self>, TablebaseError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(TablebaseError::EmptyPath);
        }
        // The library marks itself initialized even when it then finds nothing
        // to probe and can never be initialized again, so the directories are
        // checked first.
        let found = find_tables(path);
        if found.wdl_files == 0 {
            return Err(TablebaseError::NoTables);
        }

        let mut active = lock(&ACTIVE);
        if let Some(existing) = active.upgrade() {
            // Dropping a set takes this lock, and the set may turn out to be
            // the last one holding it, so the guard has to go first.
            drop(active);
            return if existing.path == path {
                Ok(existing)
            } else {
                Err(TablebaseError::InUse)
            };
        }

        let probe = TableBases::<Adapter>::new(path).map_err(|error| match error {
            TBError::BadPath => TablebaseError::NoTables,
            TBError::AlreadyInitialized | TBError::NotSingleton => TablebaseError::InUse,
            TBError::InitFailed | TBError::ProbeFailed => TablebaseError::InitFailed,
        })?;
        let tables = Arc::new(Self {
            summary: found,
            probe: Some(probe),
            path: path.to_owned(),
            root_probe: Mutex::new(()),
        });
        *active = Arc::downgrade(&tables);
        Ok(tables)
    }

    pub(super) fn path(&self) -> &str {
        &self.path
    }

    pub(super) fn summary(&self) -> TablebaseSummary {
        self.summary
    }

    /// Most pieces, kings included, that a position can have and be probed.
    pub(super) fn largest(&self) -> usize {
        self.summary.largest
    }

    /// Probes the win-draw-loss tables for the side to move.
    ///
    /// The table ignores the halfmove clock, so the answer holds for a position
    /// only as far as a zeroing move is still allowed. Positions with castling
    /// rights, or more pieces than the tables hold, have no answer.
    pub(super) fn probe_wdl(&self, board: &Board) -> Option<Wdl> {
        if has_castling_rights(board) || board.occupied().len() as usize > self.summary.largest {
            return None;
        }
        let bits = Bits::of(board);
        self.probe
            .as_ref()?
            .probe_wdl(
                bits.white,
                bits.black,
                bits.kings,
                bits.queens,
                bits.rooks,
                bits.bishops,
                bits.knights,
                bits.pawns,
                bits.en_passant,
                bits.white_to_move,
            )
            .ok()
            .map(Wdl::from)
    }

    /// Ranks the moves of a root position the tables cover and keeps those of
    /// the best rank.
    ///
    /// Distance-to-zero tables rank the moves as Stockfish does: every move
    /// that keeps a win inside the fifty-move rule ranks alike, so the search
    /// chooses among them, while a move that lets the win run out ranks below.
    /// Once a position since the last zeroing move has occurred twice, `repeated`,
    /// the wins rank by their distance instead, so that a search which could
    /// otherwise shuffle until the game is drawn is made to make progress.
    /// Without distance files the win-draw-loss tables rank by result alone.
    pub(super) fn rank_root(
        &self,
        board: &Board,
        moves: &[Move],
        use_rule_50: bool,
        repeated: bool,
    ) -> Option<RootRanking> {
        if moves.is_empty()
            || has_castling_rights(board)
            || board.occupied().len() as usize > self.summary.largest
        {
            return None;
        }
        let ranks = self
            .rank_by_distance(board, moves, repeated)
            .or_else(|| self.rank_by_result(board, moves, use_rule_50))?;
        let best = *ranks.iter().max()?;
        Some(RootRanking {
            moves: moves
                .iter()
                .zip(&ranks)
                .filter(|&(_, &rank)| rank == best)
                .map(|(&chess_move, _)| chess_move)
                .collect(),
            score: rank_score(best, use_rule_50),
        })
    }

    fn rank_by_distance(&self, board: &Board, moves: &[Move], repeated: bool) -> Option<Vec<i32>> {
        let bits = Bits::of(board);
        let result = {
            let _serialized = lock(&self.root_probe);
            self.probe
                .as_ref()?
                .probe_root(
                    bits.white,
                    bits.black,
                    bits.kings,
                    bits.queens,
                    bits.rooks,
                    bits.bishops,
                    bits.knights,
                    bits.pawns,
                    u32::from(board.halfmove_clock()),
                    bits.en_passant,
                    bits.white_to_move,
                )
                .ok()?
        };
        let halfmove_clock = i32::from(board.halfmove_clock());
        moves
            .iter()
            .map(|&chess_move| {
                let found =
                    result.moves[..result.num_moves]
                        .iter()
                        .find_map(|value| match value {
                            DtzProbeValue::DtzResult(found) if names_move(found, chess_move) => {
                                Some(*found)
                            }
                            _ => None,
                        })?;
                let distance = i32::from(found.dtz);
                let signed = match found.wdl {
                    WdlProbeResult::Win | WdlProbeResult::CursedWin => distance,
                    WdlProbeResult::Loss | WdlProbeResult::BlessedLoss => -distance,
                    WdlProbeResult::Draw => 0,
                };
                Some(distance_rank(signed, halfmove_clock, repeated))
            })
            .collect()
    }

    fn rank_by_result(&self, board: &Board, moves: &[Move], use_rule_50: bool) -> Option<Vec<i32>> {
        moves
            .iter()
            .map(|&chess_move| {
                let mut child = board.clone();
                child.play_unchecked(chess_move);
                let result = match self.probe_wdl(&child)?.reversed() {
                    Wdl::CursedWin if !use_rule_50 => Wdl::Win,
                    Wdl::BlessedLoss if !use_rule_50 => Wdl::Loss,
                    other => other,
                };
                Some(match result {
                    Wdl::Win => MAX_DTZ,
                    Wdl::CursedWin => MAX_DTZ - 101,
                    Wdl::Draw => 0,
                    Wdl::BlessedLoss => -MAX_DTZ + 101,
                    Wdl::Loss => -MAX_DTZ,
                })
            })
            .collect()
    }
}

fn has_castling_rights(board: &Board) -> bool {
    [Color::White, Color::Black].into_iter().any(|color| {
        let rights = board.castle_rights(color);
        rights.short.is_some() || rights.long.is_some()
    })
}

fn names_move(found: &pyrrhic_rs::DtzResult, chess_move: Move) -> bool {
    let promotion = match found.promotion {
        pyrrhic_rs::Piece::Queen => Some(Piece::Queen),
        pyrrhic_rs::Piece::Rook => Some(Piece::Rook),
        pyrrhic_rs::Piece::Bishop => Some(Piece::Bishop),
        pyrrhic_rs::Piece::Knight => Some(Piece::Knight),
        pyrrhic_rs::Piece::Pawn | pyrrhic_rs::Piece::King => None,
    };
    usize::from(found.from_square) == chess_move.from as usize
        && usize::from(found.to_square) == chess_move.to as usize
        && promotion == chess_move.promotion
}

/// How good a root move is, given the signed distance to a zeroing move that
/// its resulting position has for the side that played it.
///
/// A win that fits inside the fifty-move rule ranks highest, and equally
/// wherever it lies, unless a position has repeated. Any other win ranks by the
/// budget it uses, its distance plus the halfmove clock, so a shorter one is
/// preferred. Losses rank the other way round, and draws in between.
fn distance_rank(distance: i32, halfmove_clock: i32, repeated: bool) -> i32 {
    if distance > 0 {
        if distance + halfmove_clock <= 99 && !repeated {
            MAX_DTZ
        } else {
            MAX_DTZ - (distance + halfmove_clock)
        }
    } else if distance < 0 {
        if -distance * 2 + halfmove_clock < 100 {
            -MAX_DTZ
        } else {
            -MAX_DTZ + (-distance + halfmove_clock)
        }
    } else {
        0
    }
}

/// The score a rank stands for: a tablebase win or loss for the certain ones,
/// and a few centipawns for a result the fifty-move rule still decides.
fn rank_score(rank: i32, use_rule_50: bool) -> Score {
    let bound = if use_rule_50 { MAX_DTZ - 100 } else { 1 };
    if rank >= bound {
        TB_WIN_SCORE
    } else if rank > 0 {
        (rank - (MAX_DTZ - 200)).max(3) * 100 / 200
    } else if rank == 0 {
        0
    } else if rank > -bound {
        (rank + (MAX_DTZ - 200)).min(-3) * 100 / 200
    } else {
        -TB_WIN_SCORE
    }
}

/// Lists the tables of a path: how many of each kind, and the most pieces any
/// of them covers.
///
/// The piece count comes from the file names, `KRPvKR` being five pieces. The
/// probing library reports its own largest size, but it says seven whatever
/// files it was given.
fn find_tables(path: &str) -> TablebaseSummary {
    let mut found = TablebaseSummary::default();
    // The library splits the list on colons whatever the platform.
    for directory in path.split(':').filter(|directory| !directory.is_empty()) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry = entry.path();
            match entry.extension().and_then(|extension| extension.to_str()) {
                Some("rtbw") => {
                    found.wdl_files += 1;
                    let pieces = entry
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map_or(0, |stem| stem.chars().filter(|&c| c != 'v').count());
                    found.largest = found.largest.max(pieces);
                }
                Some("rtbz") => found.dtz_files += 1,
                _ => {}
            }
        }
    }
    found
}

/// The tablebase settings of an engine.
#[derive(Clone, Debug)]
pub(super) struct Configuration {
    tables: Option<Arc<Tablebases>>,
    probe_depth: u32,
    use_rule_50: bool,
    probe_limit: u32,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            tables: None,
            probe_depth: DEFAULT_SYZYGY_PROBE_DEPTH,
            use_rule_50: true,
            probe_limit: DEFAULT_SYZYGY_PROBE_LIMIT,
        }
    }
}

impl Configuration {
    /// What a search needs to probe, or `None` when it never will.
    pub(super) fn settings(&self) -> Option<TablebaseSettings<'_>> {
        let tables = self.tables.as_deref()?;
        let limit = tables.largest().min(self.probe_limit as usize);
        (limit > 0).then_some(TablebaseSettings {
            tables,
            limit,
            probe_depth: self.probe_depth,
            use_rule_50: self.use_rule_50,
        })
    }
}

/// How a search probes the tablebases.
#[derive(Clone, Copy, Debug)]
pub(super) struct TablebaseSettings<'tables> {
    pub(super) tables: &'tables Tablebases,
    /// Positions with more pieces than this are never probed.
    pub(super) limit: usize,
    /// Fewest plies of remaining depth at which a position with exactly
    /// `limit` pieces is probed.
    pub(super) probe_depth: u32,
    /// Whether a win or loss the fifty-move rule cancels counts as a draw.
    pub(super) use_rule_50: bool,
}

impl TablebaseSettings<'_> {
    /// Whether the search may ask the tables about this node.
    ///
    /// Only a position whose halfmove clock is zero is asked, which is where
    /// the tables' answer is exact.
    pub(super) fn covers(&self, board: &Board, depth: u32) -> bool {
        let pieces = board.occupied().len() as usize;
        pieces <= self.limit
            && (pieces < self.limit || depth >= self.probe_depth)
            && board.halfmove_clock() == 0
            && !has_castling_rights(board)
    }
}

impl Engine {
    /// Returns the path the tablebases were loaded from, if any are loaded.
    #[must_use]
    pub fn syzygy_path(&self) -> Option<&str> {
        self.tablebase.tables.as_deref().map(Tablebases::path)
    }

    /// Returns what the loaded tablebases hold, or nothing when none are.
    #[must_use]
    pub fn syzygy_summary(&self) -> TablebaseSummary {
        self.tablebase
            .tables
            .as_deref()
            .map(Tablebases::summary)
            .unwrap_or_default()
    }

    /// Loads the tablebases found in `path` in place of any loaded before.
    ///
    /// The path lists directories separated by colons. An empty path or
    /// [`NO_SYZYGY_PATH`] only removes the tablebases. The old set is released
    /// first because the probing library holds one at a time, so a failed load
    /// leaves the engine without tablebases.
    pub fn set_syzygy_path(&mut self, path: &str) -> Result<TablebaseSummary, TablebaseError> {
        self.tablebase.tables = None;
        let path = path.trim();
        if path.is_empty() || path == NO_SYZYGY_PATH {
            return Ok(TablebaseSummary::default());
        }
        let tables = Tablebases::load(path)?;
        let summary = tables.summary();
        self.tablebase.tables = Some(tables);
        Ok(summary)
    }

    /// Returns the fewest plies of remaining depth at which a position with as
    /// many pieces as the probe limit allows is probed inside the search.
    #[must_use]
    pub fn syzygy_probe_depth(&self) -> u32 {
        self.tablebase.probe_depth
    }

    /// Changes the probe depth, clamped to the accepted range.
    pub fn set_syzygy_probe_depth(&mut self, depth: u32) {
        self.tablebase.probe_depth = depth.clamp(MIN_SYZYGY_PROBE_DEPTH, MAX_SYZYGY_PROBE_DEPTH);
    }

    /// Whether a win the fifty-move rule cancels counts as a draw.
    #[must_use]
    pub fn syzygy_rule_50(&self) -> bool {
        self.tablebase.use_rule_50
    }

    /// Changes whether the fifty-move rule cancels wins and losses.
    pub fn set_syzygy_rule_50(&mut self, enabled: bool) {
        self.tablebase.use_rule_50 = enabled;
    }

    /// Returns the most pieces a probed position may have.
    #[must_use]
    pub fn syzygy_probe_limit(&self) -> u32 {
        self.tablebase.probe_limit
    }

    /// Changes the most pieces a probed position may have, clamped to the
    /// accepted range. The tables' own size still narrows it.
    pub fn set_syzygy_probe_limit(&mut self, limit: u32) {
        self.tablebase.probe_limit = limit.min(MAX_SYZYGY_PROBE_LIMIT);
    }
}
