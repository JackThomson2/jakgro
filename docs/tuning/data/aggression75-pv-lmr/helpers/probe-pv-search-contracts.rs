use jakgro::engine::{Engine, Position, SearchLimits};

fn main() {
    let opening = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    for aggression in [0, 75, 100] {
        let mut engine = Engine::new();
        engine.set_aggression(aggression);
        engine.set_hash_size_mib(1).unwrap();
        engine.set_position(Position::from_fen(opening).unwrap());
        for pass in 0..3 {
            let result = engine.search(&SearchLimits { depth: Some(5), ..SearchLimits::default() });
            println!("warm {aggression} {pass}: move={:?} info={:?}", result.best_move(), result.info());
        }
    }
    for line in include_str!(concat!(env!("PWD"), "/tests/data/null-move-contract.epd")).lines() {
        if line.trim().is_empty() || line.starts_with('#') { continue; }
        let fen = line.split(';').next().unwrap().trim();
        let id = line.split(';').find(|f| f.trim().starts_with("id ")).unwrap();
        for depth in [7, 9] {
            for enabled in [false, true] {
                let mut engine = Engine::new();
                engine.set_aggression(0);
                engine.set_position(Position::from_fen(fen).unwrap());
                let result = engine.search(&SearchLimits { depth: Some(depth), null_move: Some(enabled), ..SearchLimits::default() });
                println!("{id} depth={depth} null={enabled}: move={:?} info={:?}", result.best_move(), result.info());
            }
        }
    }
}
