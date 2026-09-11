use cozy_chess::Board;
use jakgro::engine::tuning::{BlockKind, BLOCKS, FEATURE_COUNT, TuningPosition, current_weights, tuning_features};
use std::{env, fs};

fn probability(score: f64, k: f64) -> f64 { 1.0 / (1.0 + 10.0_f64.powf(-score * k / 400.0)) }
fn weights(path: &str) -> Vec<(i32, i32)> {
    let mut weights = Vec::new();
    for line in fs::read_to_string(path).unwrap().lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].parse::<usize>().unwrap(), weights.len());
        weights.push((fields[1].parse().unwrap(), fields[2].parse().unwrap()));
    }
    assert_eq!(weights.len(), FEATURE_COUNT);
    weights
}
fn samples(path: &str) -> Vec<(TuningPosition, f64)> {
    fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|line| {
        let fields: Vec<_> = line.split(';').collect();
        let board: Board = fields[0].parse().unwrap();
        let outcome: f64 = fields[1].parse().unwrap();
        assert!(outcome.is_finite() && (0.0..=1.0).contains(&outcome));
        (tuning_features(&board), outcome)
    }).collect()
}
fn mse(scores: &[(f64, f64)], k: f64) -> f64 {
    scores.iter().map(|&(s,y)| (probability(s,k)-y).powi(2)).sum::<f64>() / scores.len() as f64
}
fn main() {
    let a: Vec<String> = env::args().collect();
    match a[1].as_str() {
        "layout" => {
            println!("kind\tname\toffset\tlen");
            for b in BLOCKS {
                let kind = match b.kind { BlockKind::Scalar=>"scalar", BlockKind::Array=>"array", BlockKind::Table=>"table" };
                println!("{kind}\t{}\t{}\t{}",b.name,b.offset,b.len);
            }
        }
        "weights" => {
            println!("index\tmg\teg");
            for (i,(mg,eg)) in current_weights().iter().enumerate() { println!("{i}\t{mg}\t{eg}"); }
        }
        "calibrate" | "evaluate" => {
            let rows = samples(&a[2]);
            assert!(!rows.is_empty());
            let weights = weights(&a[3]);
            let scores: Vec<_> = rows.iter().map(|(p,y)| (p.score(&weights) as f64,*y)).collect();
            let k = if a[1] == "calibrate" {
                let (mut low,mut high)=(0.1_f64,10.0_f64);
                for _ in 0..60 {
                    let first=low+(high-low)/3.0;
                    let second=high-(high-low)/3.0;
                    if mse(&scores,first)<mse(&scores,second) { high=second; } else { low=first; }
                }
                let k=(low+high)/2.0;
                fs::write(&a[4],format!("{k:.17}\n")).unwrap();
                k
            } else { fs::read_to_string(&a[4]).unwrap().trim().parse::<f64>().unwrap() };
            assert!(k.is_finite() && k>0.0);
            let mut counts=[0_usize;3];
            let mut losses=[0.0_f64;3];
            let (mut min,mut max)=(i32::MAX,i32::MIN);
            for ((p,y),(score,_)) in rows.iter().zip(&scores) {
                let bucket=if p.phase<=8 { 0 } else if p.phase>=16 { 2 } else { 1 };
                counts[bucket]+=1;
                losses[bucket]+=(probability(*score,k)-y).powi(2);
                min=min.min(*score as i32); max=max.max(*score as i32);
            }
            let per_phase=(0..3).map(|i| if counts[i]==0 { "null".to_owned() } else { format!("{}",losses[i]/counts[i] as f64) }).collect::<Vec<_>>().join(",");
            println!("{{\"positions\":{},\"k\":{},\"mse\":{},\"phase_counts_eg_mixed_mg\":{:?},\"phase_mse\":[{}],\"min_score\":{},\"max_score\":{}}}",rows.len(),k,mse(&scores,k),counts,per_phase,min,max);
        }
        _ => panic!("expected layout, weights, calibrate or evaluate"),
    }
}
