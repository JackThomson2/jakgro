use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Deterministic arithmetic fixtures, not trained playing networks.
///
/// The unpatterned network has zero weights and an output bias of `score`
/// centipawns: 10_404 numerator units per centipawn, since the score is
/// `numerator * 400 / (64 * 255 * 255)`. Its evaluation is `score` from any
/// position and either side. The patterned network adds small input weights
/// and output weights across the admitted `-127..=127` range.
pub fn network_bytes(score: i32, patterned: bool) -> Vec<u8> {
    const HIDDEN: usize = 512;
    const INPUTS: usize = 6_144;
    const BUCKETS: usize = 8;
    const PAYLOAD: usize = 2 * (HIDDEN + INPUTS * HIDDEN + BUCKETS * 2 * HIDDEN) + 4 * BUCKETS;
    let mut bytes = Vec::with_capacity(48 + PAYLOAD);
    bytes.extend_from_slice(b"JAKNNUE\0");
    for value in [
        3_u32,
        1,
        INPUTS as u32,
        HIDDEN as u32,
        255,
        64,
        PAYLOAD as u32,
        BUCKETS as u32,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for _ in 0..HIDDEN {
        bytes.extend_from_slice(&128_i16.to_le_bytes());
    }
    for index in 0..INPUTS * HIDDEN {
        let weight = if patterned {
            ((index * 37 + index / HIDDEN) % 31) as i16 - 15
        } else {
            0
        };
        bytes.extend_from_slice(&weight.to_le_bytes());
    }
    for _ in 0..BUCKETS {
        for index in 0..2 * HIDDEN {
            let weight = if patterned {
                ((index * 71 + 13) % 255) as i16 - 127
            } else {
                0
            };
            bytes.extend_from_slice(&weight.to_le_bytes());
        }
    }
    for _ in 0..BUCKETS {
        bytes.extend_from_slice(&(score * 10_404).to_le_bytes());
    }
    let mut checksum = 14_695_981_039_346_656_037_u64;
    for &byte in &bytes[48..] {
        checksum = (checksum ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
    }
    bytes[40..48].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

pub struct NetworkFile {
    directory: PathBuf,
    pub path: PathBuf,
}

impl NetworkFile {
    pub fn new(score: i32, patterned: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "jakgro-network-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("network  value λ.nnue");
        std::fs::write(&path, network_bytes(score, patterned)).unwrap();
        Self { directory, path }
    }
}

impl Drop for NetworkFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
