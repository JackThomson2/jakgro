//! Prints every `SearchTelemetry` counter for fixed-depth searches over a book.
//!
//! ```text
//! cargo run --release --example search_telemetry -- BOOK DEPTH COUNT AGGRESSION [HASH_MIB]
//! ```
//!
//! `BOOK` is an EPD file; the first four fields of every line are searched as a
//! position. `COUNT` positions are taken at a fixed stride through the book so
//! the sample is spread over it. One line per position reports the completed
//! depth, the node count, the wall time and the `Debug` rendering of the
//! telemetry, which can be summed over positions to read quiescence share,
//! table hit rates, null-move and late-move-reduction statistics, aspiration
//! failures and the personality's root work.

use std::error::Error;
use std::fs;
use std::time::Instant;

use jakgro::engine::{Engine, Position, SearchLimits};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, book, depth, count, aggression, rest @ ..] = args.as_slice() else {
        return Err("usage: search_telemetry BOOK DEPTH COUNT AGGRESSION [HASH_MIB]".into());
    };
    let depth: u32 = depth.parse()?;
    let count: usize = count.parse()?;
    let aggression: u8 = aggression.parse()?;
    let hash_mib: usize = rest.first().map_or(Ok(16), |value| value.parse())?;

    let fens: Vec<String> = fs::read_to_string(book)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(';').next()?.split_whitespace().collect();
            (fields.len() >= 4).then(|| format!("{} 0 1", fields[..4].join(" ")))
        })
        .collect();
    let stride = (fens.len() / count.max(1)).max(1);

    for fen in fens.iter().step_by(stride).take(count) {
        let mut engine = Engine::new();
        engine.set_aggression(aggression);
        engine.set_hash_size_mib(hash_mib)?;
        engine.set_position(Position::from_fen(fen)?);
        let started = Instant::now();
        let result = engine.search(&SearchLimits {
            depth: Some(depth),
            ..SearchLimits::default()
        });
        let elapsed = started.elapsed().as_millis();
        let Some(info) = result.info() else {
            println!("FEN {fen} | no completed iteration");
            continue;
        };
        println!(
            "FEN {fen} | depth {} nodes {} ms {elapsed} | {:?}",
            info.depth(),
            info.nodes(),
            result.telemetry()
        );
    }
    Ok(())
}
