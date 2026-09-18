//! Integer kernels of the feature transformer and output layer.
//!
//! The arithmetic is identical on every path; the only difference is the
//! vector width the compiler may use. A wider path is selected at runtime when
//! the CPU supports it, so the shipped binary keeps its baseline target while
//! the evaluation runs at the host's width. Selection happens once per
//! perspective update and once per output, never per feature.

use std::ops::{Deref, DerefMut};

use super::{ACTIVATION_MAX, HIDDEN_SIZE};

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
    if is_x86_feature_detected!("avx2") {
        // SAFETY: the feature check above guarantees the CPU executes AVX2;
        // the function has no other requirements.
        return unsafe { avx2::apply(weights, target, source, adds, subs) };
    }
    apply_impl(weights, target, source, adds, subs);
}

/// The output numerator without its bias: both clipped perspectives dotted
/// with their weights. The loader proves the total fits an `i32`.
pub(super) fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i32 {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        // SAFETY: as in `apply`.
        return unsafe { avx2::output(weights, sums) };
    }
    output_impl(weights, sums)
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

#[inline(always)]
fn output_impl(weights: &[Row; 2], sums: [&Row; 2]) -> i32 {
    let mut total = 0;
    for (weights, sums) in weights.iter().zip(sums) {
        for (&weight, &sum) in weights.0.iter().zip(&sums.0) {
            total += i32::from(weight) * i32::from(sum.clamp(0, ACTIVATION_MAX as i16));
        }
    }
    total
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::*;

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
    pub(super) unsafe fn output(weights: &[Row; 2], sums: [&Row; 2]) -> i32 {
        output_impl(weights, sums)
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

    #[test]
    fn output_clips_each_activation_before_the_signed_product() {
        let weights = [
            Row(std::array::from_fn(|unit| {
                (unit as i32 * 509 - 32_000) as i16
            })),
            Row([i16::MIN; HIDDEN_SIZE]),
        ];
        let sums = [
            Row(std::array::from_fn(|unit| (unit as i16 * 7) - 300)),
            Row(std::array::from_fn(|unit| 400 - unit as i16 * 5)),
        ];
        let expected: i64 = weights
            .iter()
            .zip(&sums)
            .flat_map(|(weights, sums)| weights.0.iter().zip(&sums.0))
            .map(|(&weight, &sum)| i64::from(weight) * i64::from(sum.clamp(0, 255)))
            .sum();
        assert_eq!(i64::from(output(&weights, [&sums[0], &sums[1]])), expected);
    }
}
