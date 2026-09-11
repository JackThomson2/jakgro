use cozy_chess::{Board, GameStatus, Move, Piece};
use std::{collections::{HashMap, HashSet}, env, fs, io::{BufWriter, Write}, path::Path};

fn moves(board: &Board) -> Vec<Move> {
    let mut all = Vec::new();
    board.generate_moves(|piece| { all.extend(piece); false });
    all
}
fn en_passant(board: &Board, m: Move) -> bool {
    board.piece_on(m.from) == Some(Piece::Pawn)
        && m.from.file() != m.to.file() && board.piece_on(m.to).is_none()
}
fn canonical(board: &Board) -> String {
    let fen = board.to_string();
    let mut fields: Vec<_> = fen.split_whitespace().take(4).collect();
    if board.en_passant().is_some() && !moves(board).into_iter().any(|m| en_passant(board, m)) {
        fields[3] = "-";
    }
    fields.join(" ")
}
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    (x ^ (x >> 31)).max(1)
}
fn next(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}
fn channel(name: &str) -> &'static str {
    match name {
        "french-advance" | "french-tarrasch" | "french-exchange" | "caro-advance" |
        "caro-classical" | "pirc" | "modern" | "owen" => "development",
        "english-symmetrical" | "english-reversed-sicilian" | "english-four-knights" |
        "reti-kingside" | "reti-c4" | "larsen" | "bird" | "polish" => "confirmation",
        _ => "training",
    }
}
fn main() {
    let args: Vec<String> = env::args().collect();
    if args[1] == "keys" {
        let input = fs::read_to_string(&args[2]).unwrap();
        let mut output = BufWriter::new(fs::File::create(&args[3]).unwrap());
        for (i, line) in input.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') { continue; }
            let fen = line.split(';').next().unwrap();
            let board: Board = fen.parse().unwrap_or_else(|e| panic!("line {}: {e:?}: {fen}", i + 1));
            writeln!(output, "{}", canonical(&board)).unwrap();
        }
        return;
    }
    assert_eq!(args[1], "generate");
    let parents_text = fs::read_to_string(&args[2]).unwrap();
    let parents: Vec<(String, Board)> = parents_text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|line| {
        let (fen, label) = line.split_once(" id ").unwrap();
        let name = label.trim().strip_prefix('"').unwrap().strip_suffix("\";").unwrap().to_owned();
        let board: Board = format!("{fen} 0 1").parse().unwrap();
        assert!(board.checkers().is_empty() && board.status() == GameStatus::Ongoing);
        (name, board)
    }).collect();
    assert_eq!(parents.len(), 48);
    let parent_channels: HashMap<String, &str> = parents.iter().map(|(name, b)| (canonical(b), channel(name))).collect();
    let mut seen = HashSet::new();
    for (i, line) in fs::read_to_string(&args[3]).unwrap().lines().enumerate() {
        let board: Board = line.parse().unwrap_or_else(|e| panic!("historical line {}: {e:?}: {line}", i+1));
        seen.insert(canonical(&board));
    }
    for (_, b) in &parents { seen.insert(canonical(b)); }
    let out = Path::new(&args[4]);
    fs::create_dir_all(out).unwrap();
    let mut initial: Vec<_> = seen.iter().cloned().collect();
    initial.sort();
    fs::write(out.join("historical.keys"), initial.join("\n") + "\n").unwrap();
    let mut lineage = BufWriter::new(fs::File::create(out.join("lineage.tsv")).unwrap());
    writeln!(lineage, "channel\tparent\tround\tattempt\tseed\tplies\tmoves\tactual_fen\tcanonical").unwrap();
    for (kind, quota, master, expected_parents) in [
        ("training", 64, 0x75202609110001_u64, 32),
        ("development", 64, 0x75202609110002_u64, 8),
        ("confirmation", 192, 0x75202609110003_u64, 8),
    ] {
        let members: Vec<_> = parents.iter().enumerate().filter(|(_, (name, _))| channel(name) == kind).collect();
        assert_eq!(members.len(), expected_parents);
        let mut attempts = vec![0_u64; parents.len()];
        let mut book = BufWriter::new(fs::File::create(out.join(format!("{kind}.epd"))).unwrap());
        let mut keys = BufWriter::new(fs::File::create(out.join(format!("{kind}.keys"))).unwrap());
        let (mut accepted, mut collisions, mut exhausted) = (0, 0, 0);
        for round in 0..quota {
            for &(index, (name, parent)) in &members {
                loop {
                    attempts[index] += 1;
                    assert!(attempts[index] < 100_000, "exhausted {kind}/{name}");
                    let seed = mix(master ^ ((index as u64) << 32) ^ attempts[index]);
                    let mut rng = seed;
                    let plies = 8 + (next(&mut rng) % 9) as usize;
                    let mut board = parent.clone();
                    let mut path = HashSet::from([canonical(&board)]);
                    let mut played = Vec::new();
                    for _ in 0..plies {
                        let mut legal = Vec::new();
                        for m in moves(&board) {
                            if m.promotion.is_some() || board.piece_on(m.to).is_some() || en_passant(&board, m) { continue; }
                            let mut child = board.clone();
                            child.play_unchecked(m);
                            if !child.checkers().is_empty() || child.status() != GameStatus::Ongoing { continue; }
                            let key = canonical(&child);
                            if path.contains(&key) || parent_channels.get(&key).is_some_and(|other| *other != kind) { continue; }
                            legal.push((m.to_string(), m, child, key));
                        }
                        legal.sort_by(|a,b| a.0.cmp(&b.0));
                        if legal.is_empty() { break; }
                        let chosen = (next(&mut rng) % legal.len() as u64) as usize;
                        let (_, m, child, key) = legal.swap_remove(chosen);
                        board = child;
                        played.push(m);
                        path.insert(key);
                    }
                    if played.len() != plies { exhausted += 1; continue; }
                    let key = canonical(&board);
                    if !seen.insert(key.clone()) { collisions += 1; continue; }
                    let mut replay = parent.clone();
                    for &m in &played { assert!(replay.is_legal(m)); replay.play_unchecked(m); }
                    assert_eq!(replay.to_string(), board.to_string());
                    let id = format!("refit-v1-{kind}-{name}-{round:03}-{:05}", attempts[index]);
                    writeln!(book, "{key} id \"{id}\";").unwrap();
                    writeln!(keys, "{key}").unwrap();
                    let uci = played.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
                    writeln!(lineage, "{kind}\t{name}\t{round}\t{}\t{seed:016x}\t{plies}\t{uci}\t{board}\t{key}", attempts[index]).unwrap();
                    accepted += 1;
                    break;
                }
            }
        }
        println!("{kind}: parents={} positions={accepted} attempts={} endpoint_collisions={collisions} incomplete_paths={exhausted}; all prefixes replayed", members.len(), attempts.iter().sum::<u64>());
    }
}
