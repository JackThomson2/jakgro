//! Integer kernels of the feature transformer and output layer.
//!
//! Every path computes the same exact integers; they differ only in vector
//! width and in the order the exact partial sums are added. The widest path
//! the CPU supports is selected at runtime, so the shipped binary keeps its
//! baseline target while the evaluation runs at the host's width. Selection
//! happens once per perspective update and once per output, never per
//! feature.

use std::ops::{Deref, DerefMut};

use super::{ACTIVATION_MAX, HIDDEN_SIZE, OUTPUT_WEIGHT_LIMIT};

/// One hidden-width vector: a feature's weights or a perspective's sums.
///
/// Rows start on a cache line and end on one, so a row never shares a line
/// with its neighbour and vector loads never straddle two lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(64))]
pub(super) struct Row(pub [i16; HIDDEN_SIZE]);

const _: () = assert!(size_of::<Row>() == 2 * HIDDEN_SIZE);

impl Deref for Row {
    type Target = [i16; HIDDEN_SIZE];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Row {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Sets `target` to `source + adds - subs`, or updates it in place without a
/// `source`.
///
/// Sums wrap. The loader proves every real placement's sums fit an `i16`, and
/// wrapping addition is exact modulo 2^16, so the result is exact whenever it
/// describes a placement, whatever the intermediate values were.
pub(super) fn apply(
    weights: &[Row],
    target: &mut Row,
    source: Option<&Row>,
    adds: &[u16],
    subs: &[u16],
) {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: each feature check guarantees the CPU executes the
        // instructions its function enables; they have no other requirements.
        if is_x86_feature_detected!("avx512bw") {
            return unsafe { avx512::apply(weights, target, source, adds, subs) };
        }
        if is_x86_feature_detected!("avx2") {
            return unsafe { avx2::apply(weights, target, source, adds, subs) };
        }
    }
    apply_impl(weights, target, source, adds, subs);
}

/// The output numerator without its bias: both perspectives' squared clipped
/// activations dotted with their weights.
///
/// Requires every weight within `-OUTPUT_WEIGHT_LIMIT..=OUTPUT_WEIGHT_LIMIT`,
/// which the loader enforces: then `weight * a` fits an `i16`, and
/// [`OUTPUT_CHUNK`] products fit an `i32`, so the sum is exact by construction
/// rather than by any property of the trained values. Every kernel lets no
/// `i32` accumulate more than `OUTPUT_CHUNK` products before widening.
pub(super) fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: as in `apply`.
        if is_x86_feature_detected!("avx512bw") {
            return unsafe { avx512::output(weights, sums) };
        }
        if is_x86_feature_detected!("avx2") {
            return unsafe { avx2::output(weights, sums) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is part of the AArch64 baseline.
    return unsafe { neon::output(weights, sums) };
    #[cfg(not(target_arch = "aarch64"))]
    output_impl(weights, sums)
}

/// Products one `i32` accumulates before spilling into the `i64` total:
/// `64 * 127 * 255 * 255 < 2^31`.
const OUTPUT_CHUNK: usize = 64;
const _: () = assert!(HIDDEN_SIZE.is_multiple_of(OUTPUT_CHUNK));
const _: () = assert!(
    OUTPUT_CHUNK as i64
        * OUTPUT_WEIGHT_LIMIT as i64
        * ACTIVATION_MAX as i64
        * ACTIVATION_MAX as i64
        <= i32::MAX as i64
);
const _: () = assert!(OUTPUT_WEIGHT_LIMIT as i32 * ACTIVATION_MAX <= i16::MAX as i32);

/// Whether `lanes` independent `i32` accumulators covering `units` units
/// between them each take at most [`OUTPUT_CHUNK`] products.
const fn lanes_within_chunk(lanes: usize, units: usize) -> bool {
    units.is_multiple_of(lanes) && units / lanes <= OUTPUT_CHUNK
}

/// One pass adds at most two rows and subtracts at most two. Ordinary moves
/// change two to four features of a perspective, which is one pass. A rebuild
/// only adds, four rows to a pass.
#[inline(always)]
fn apply_impl(
    weights: &[Row],
    target: &mut Row,
    mut source: Option<&Row>,
    mut adds: &[u16],
    mut subs: &[u16],
) {
    let row = |index: u16| &weights[usize::from(index)];
    loop {
        let from = source.take();
        match (adds, subs) {
            ([], []) => {
                if let Some(from) = from {
                    *target = *from;
                }
                return;
            }
            ([a], []) => return combine(target, from, [row(*a)], []),
            ([], [s]) => return combine(target, from, [], [row(*s)]),
            ([a], [s]) => return combine(target, from, [row(*a)], [row(*s)]),
            ([a], [s, t, rest @ ..]) => {
                combine(target, from, [row(*a)], [row(*s), row(*t)]);
                (adds, subs) = (&[], rest);
            }
            ([a, b, rest @ ..], [s]) => {
                combine(target, from, [row(*a), row(*b)], [row(*s)]);
                (adds, subs) = (rest, &[]);
            }
            ([a, b, c, d, rest @ ..], []) => {
                combine(target, from, [row(*a), row(*b), row(*c), row(*d)], []);
                adds = rest;
            }
            ([a, b, rest @ ..], []) => {
                combine(target, from, [row(*a), row(*b)], []);
                adds = rest;
            }
            ([], [s, t, rest @ ..]) => {
                combine(target, from, [], [row(*s), row(*t)]);
                subs = rest;
            }
            ([a, b, more @ ..], [s, t, rest @ ..]) => {
                combine(target, from, [row(*a), row(*b)], [row(*s), row(*t)]);
                (adds, subs) = (more, rest);
            }
        }
        if adds.is_empty() && subs.is_empty() {
            return;
        }
    }
}

#[inline(always)]
fn combine<const ADDS: usize, const SUBS: usize>(
    target: &mut Row,
    source: Option<&Row>,
    adds: [&Row; ADDS],
    subs: [&Row; SUBS],
) {
    let pass = |unit: usize, mut value: i16| {
        for row in adds {
            value = value.wrapping_add(row.0[unit]);
        }
        for row in subs {
            value = value.wrapping_sub(row.0[unit]);
        }
        value
    };
    match source {
        Some(source) => {
            for unit in 0..HIDDEN_SIZE {
                target.0[unit] = pass(unit, source.0[unit]);
            }
        }
        None => {
            for unit in 0..HIDDEN_SIZE {
                target.0[unit] = pass(unit, target.0[unit]);
            }
        }
    }
}

/// The portable definition: what every vector kernel must agree with. It
/// runs on hosts without a vector kernel, and in the tests of those that have.
#[cfg(any(not(target_arch = "aarch64"), test))]
fn output_impl(weights: &[Row; 2], sums: [&Row; 2]) -> i64 {
    let mut total = 0_i64;
    for (weights, sums) in weights.iter().zip(sums) {
        for (weights, sums) in weights
            .0
            .chunks_exact(OUTPUT_CHUNK)
            .zip(sums.0.chunks_exact(OUTPUT_CHUNK))
        {
            let mut chunk = 0_i32;
            for (&weight, &sum) in weights.iter().zip(sums) {
                let activation = sum.clamp(0, ACTIVATION_MAX as i16);
                // `weight * activation` is at most 127 * 255 in magnitude, so
                // the 16-bit product is exact and the 32-bit sum of 64
                // products of it with another activation cannot overflow.
                chunk += i32::from(weight * activation) * i32::from(activation);
            }
            total += i64::from(chunk);
        }
    }
    total
}

/// The vector kernels clip sixteen-bit sums, multiply by the weights in
/// sixteen bits, and use the widening multiply-add of adjacent pairs that
/// every target offers (`pmaddwd`, `smlal`) for the second activation
/// factor. The compiler does not find this shape from the scalar definition
/// on its own: it deinterleaves the pairs and pads them with zeros, at half
/// the useful width.
#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi32, _mm256_load_si256, _mm256_madd_epi16, _mm256_max_epi16,
        _mm256_min_epi16, _mm256_mullo_epi16, _mm256_set1_epi16, _mm256_setzero_si256,
    };

    use super::*;

    /// Units per vector.
    const WIDTH: usize = 16;
    /// Each perspective has its own accumulator of `WIDTH / 2` lanes.
    const _: () = assert!(lanes_within_chunk(WIDTH / 2, HIDDEN_SIZE));

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply(
        weights: &[Row],
        target: &mut Row,
        source: Option<&Row>,
        adds: &[u16],
        subs: &[u16],
    ) {
        apply_impl(weights, target, source, adds, subs);
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i64 {
        let zero = _mm256_setzero_si256();
        let ceiling = _mm256_set1_epi16(ACTIVATION_MAX as i16);
        let mut total = 0_i64;
        for (weights, sums) in weights.iter().zip(sums) {
            let mut lanes = zero;
            for unit in (0..HIDDEN_SIZE).step_by(WIDTH) {
                // SAFETY: rows are 64-byte aligned and `unit + WIDTH` stays
                // within them.
                let (sum, weight) = unsafe {
                    (
                        _mm256_load_si256(sums.0.as_ptr().add(unit).cast()),
                        _mm256_load_si256(weights.0.as_ptr().add(unit).cast()),
                    )
                };
                let activation = _mm256_min_epi16(_mm256_max_epi16(sum, zero), ceiling);
                let product = _mm256_mullo_epi16(weight, activation);
                lanes = _mm256_add_epi32(lanes, _mm256_madd_epi16(product, activation));
            }
            // SAFETY: a vector is exactly its lanes.
            let lanes: [i32; WIDTH / 2] = unsafe { std::mem::transmute::<__m256i, _>(lanes) };
            total += lanes.iter().map(|&lane| i64::from(lane)).sum::<i64>();
        }
        total
    }
}

#[cfg(target_arch = "x86_64")]
mod avx512 {
    use std::arch::x86_64::{
        __m512i, _mm512_add_epi32, _mm512_load_si512, _mm512_madd_epi16, _mm512_max_epi16,
        _mm512_min_epi16, _mm512_mullo_epi16, _mm512_set1_epi16, _mm512_setzero_si512,
    };

    use super::*;

    /// Units per vector.
    const WIDTH: usize = 32;
    /// One accumulator of `WIDTH / 2` lanes covers both perspectives.
    const _: () = assert!(lanes_within_chunk(WIDTH / 2, 2 * HIDDEN_SIZE));

    #[target_feature(enable = "avx512bw")]
    pub(super) unsafe fn apply(
        weights: &[Row],
        target: &mut Row,
        source: Option<&Row>,
        adds: &[u16],
        subs: &[u16],
    ) {
        apply_impl(weights, target, source, adds, subs);
    }

    #[target_feature(enable = "avx512bw")]
    pub(super) unsafe fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i64 {
        let zero = _mm512_setzero_si512();
        let ceiling = _mm512_set1_epi16(ACTIVATION_MAX as i16);
        let mut lanes = zero;
        for (weights, sums) in weights.iter().zip(sums) {
            for unit in (0..HIDDEN_SIZE).step_by(WIDTH) {
                // SAFETY: rows are 64-byte aligned and `unit + WIDTH` stays
                // within them.
                let (sum, weight) = unsafe {
                    (
                        _mm512_load_si512(sums.0.as_ptr().add(unit).cast()),
                        _mm512_load_si512(weights.0.as_ptr().add(unit).cast()),
                    )
                };
                let activation = _mm512_min_epi16(_mm512_max_epi16(sum, zero), ceiling);
                let product = _mm512_mullo_epi16(weight, activation);
                lanes = _mm512_add_epi32(lanes, _mm512_madd_epi16(product, activation));
            }
        }
        // SAFETY: a vector is exactly its lanes.
        let lanes: [i32; WIDTH / 2] = unsafe { std::mem::transmute::<__m512i, _>(lanes) };
        lanes.iter().map(|&lane| i64::from(lane)).sum()
    }
}

/// NEON is part of the AArch64 baseline, so there is nothing to detect; the
/// intrinsics still require the feature on their caller.
#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::{
        vaddlvq_s32, vdupq_n_s16, vdupq_n_s32, vget_low_s16, vld1q_s16, vmaxq_s16, vminq_s16,
        vmlal_high_s16, vmlal_s16, vmulq_s16,
    };

    use super::*;

    /// Units per vector.
    const WIDTH: usize = 8;
    /// Vectors per step: each keeps its own low and high accumulator, so the
    /// multiply-add latency of one chain overlaps the others.
    const STEP: usize = 2;
    /// Each perspective has `2 * STEP` accumulators of `WIDTH / 2` lanes.
    const _: () = assert!(lanes_within_chunk(STEP * WIDTH, HIDDEN_SIZE));

    #[target_feature(enable = "neon")]
    pub(super) unsafe fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i64 {
        let zero = vdupq_n_s16(0);
        let ceiling = vdupq_n_s16(ACTIVATION_MAX as i16);
        let mut total = 0_i64;
        for (weights, sums) in weights.iter().zip(sums) {
            let mut low = [vdupq_n_s32(0); STEP];
            let mut high = [vdupq_n_s32(0); STEP];
            for step in (0..HIDDEN_SIZE).step_by(STEP * WIDTH) {
                for (chain, unit) in (step..step + STEP * WIDTH).step_by(WIDTH).enumerate() {
                    // SAFETY: `unit + WIDTH` stays within the rows.
                    let (sum, weight) = unsafe {
                        (
                            vld1q_s16(sums.0.as_ptr().add(unit)),
                            vld1q_s16(weights.0.as_ptr().add(unit)),
                        )
                    };
                    let activation = vminq_s16(vmaxq_s16(sum, zero), ceiling);
                    let product = vmulq_s16(weight, activation);
                    low[chain] =
                        vmlal_s16(low[chain], vget_low_s16(product), vget_low_s16(activation));
                    high[chain] = vmlal_high_s16(high[chain], product, activation);
                }
            }
            for chain in 0..STEP {
                total += vaddlvq_s32(low[chain]) + vaddlvq_s32(high[chain]);
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Row> {
        (0..9)
            .map(|row| {
                Row(std::array::from_fn(|unit| {
                    (unit as i16 * 37 + row * 911) - 2000
                }))
            })
            .collect()
    }

    #[test]
    fn every_shape_agrees_with_the_scalar_definition_in_and_out_of_place() {
        let weights = rows();
        let start = Row(std::array::from_fn(|unit| (unit as i16 * 13) - 600));
        let indices: Vec<u16> = (0..weights.len() as u16).collect();
        for adds in 0..=5 {
            for subs in 0..=4 {
                let (adds, subs) = (&indices[..adds], &indices[5..5 + subs]);
                let expected = Row(std::array::from_fn(|unit| {
                    let mut value = start.0[unit];
                    for &index in adds {
                        value = value.wrapping_add(weights[usize::from(index)].0[unit]);
                    }
                    for &index in subs {
                        value = value.wrapping_sub(weights[usize::from(index)].0[unit]);
                    }
                    value
                }));
                let mut in_place = start;
                apply(&weights, &mut in_place, None, adds, subs);
                assert_eq!(in_place, expected, "{adds:?} {subs:?}");
                let mut copied = Row([0; HIDDEN_SIZE]);
                apply(&weights, &mut copied, Some(&start), adds, subs);
                assert_eq!(copied, expected, "{adds:?} {subs:?}");
            }
        }
    }

    #[test]
    fn sums_wrap_instead_of_trapping_and_recover_exactly() {
        let weights = [Row([i16::MAX; HIDDEN_SIZE])];
        let mut sums = Row([5; HIDDEN_SIZE]);
        apply(&weights, &mut sums, None, &[0, 0, 0], &[]);
        apply(&weights, &mut sums, None, &[], &[0, 0, 0]);
        assert_eq!(sums, Row([5; HIDDEN_SIZE]));
    }

    /// The result of every kernel this host can run, named.
    fn outputs(weights: &[Row; 2], sums: [&Row; 2]) -> Vec<(&'static str, i64)> {
        let mut results = vec![
            ("scalar", output_impl(weights, sums)),
            ("dispatched", output(weights, sums)),
        ];
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                // SAFETY: feature detected.
                results.push(("avx2", unsafe { avx2::output(weights, sums) }));
            }
            if is_x86_feature_detected!("avx512bw") {
                // SAFETY: feature detected.
                results.push(("avx512", unsafe { avx512::output(weights, sums) }));
            }
        }
        #[cfg(target_arch = "aarch64")]
        // SAFETY: NEON is part of the AArch64 baseline.
        results.push(("neon", unsafe { neon::output(weights, sums) }));
        results
    }

    #[test]
    fn output_squares_each_clipped_activation_before_the_signed_product() {
        // Weights span the whole admitted range; sums run from far below the
        // clip to far above it, so both clips and the square are exercised.
        let weights = [
            Row(std::array::from_fn(|unit| (unit as i16 * 37) % 255 - 127)),
            Row(std::array::from_fn(|unit| [127, -127][unit % 2])),
        ];
        let sums = [
            Row(std::array::from_fn(|unit| (unit as i16 * 7) - 300)),
            Row(std::array::from_fn(|unit| 400 - unit as i16 * 5)),
        ];
        let expected: i64 = weights
            .iter()
            .zip(&sums)
            .flat_map(|(weights, sums)| weights.0.iter().zip(&sums.0))
            .map(|(&weight, &sum)| {
                let activation = i64::from(sum.clamp(0, 255));
                i64::from(weight) * activation * activation
            })
            .sum();
        for (kernel, result) in outputs(&weights, [&sums[0], &sums[1]]) {
            assert_eq!(result, expected, "{kernel}");
        }
    }

    #[test]
    fn output_extremes_exceed_i32_without_overflowing() {
        let saturated = Row([i16::MAX; HIDDEN_SIZE]);
        for weight in [OUTPUT_WEIGHT_LIMIT, -OUTPUT_WEIGHT_LIMIT] {
            let weights = [Row([weight; HIDDEN_SIZE]); 2];
            let expected = 2 * HIDDEN_SIZE as i64 * i64::from(weight) * 255 * 255;
            assert!(expected.abs() > i64::from(i32::MAX));
            for (kernel, result) in outputs(&weights, [&saturated, &saturated]) {
                assert_eq!(result, expected, "{kernel}");
            }
        }
    }
}
