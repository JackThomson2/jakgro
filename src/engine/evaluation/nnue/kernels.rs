//! Integer kernels of the feature transformer and output layer.
//!
//! The arithmetic is identical on every path; the only difference is the
//! vector width the compiler may use. A wider path is selected once at runtime
//! when the CPU supports it, so the shipped binary keeps its baseline target
//! while the evaluation runs at the host's width.

use std::sync::LazyLock;

use super::{ACTIVATION_MAX, HIDDEN_SIZE};

/// The three hot loops, chosen once per process.
pub(super) struct Kernels {
    pub add: fn(&mut [i32; HIDDEN_SIZE], &[i16]),
    pub sub: fn(&mut [i32; HIDDEN_SIZE], &[i16]),
    pub dot: fn(&[i16; HIDDEN_SIZE], &[i32; HIDDEN_SIZE]) -> i32,
}

pub(super) fn kernels() -> &'static Kernels {
    static KERNELS: LazyLock<Kernels> = LazyLock::new(select);
    &KERNELS
}

#[inline(always)]
fn add_impl(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
    for (value, &weight) in sum.iter_mut().zip(row) {
        *value += i32::from(weight);
    }
}

#[inline(always)]
fn sub_impl(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
    for (value, &weight) in sum.iter_mut().zip(row) {
        *value -= i32::from(weight);
    }
}

#[inline(always)]
fn dot_impl(weights: &[i16; HIDDEN_SIZE], sums: &[i32; HIDDEN_SIZE]) -> i32 {
    let mut total = 0;
    for (&weight, &sum) in weights.iter().zip(sums) {
        total += i32::from(weight) * sum.clamp(0, ACTIVATION_MAX);
    }
    total
}

fn add_generic(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
    add_impl(sum, row);
}

fn sub_generic(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
    sub_impl(sum, row);
}

fn dot_generic(weights: &[i16; HIDDEN_SIZE], sums: &[i32; HIDDEN_SIZE]) -> i32 {
    dot_impl(weights, sums)
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::*;

    #[target_feature(enable = "avx2")]
    unsafe fn add(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
        add_impl(sum, row);
    }

    #[target_feature(enable = "avx2")]
    unsafe fn sub(sum: &mut [i32; HIDDEN_SIZE], row: &[i16]) {
        sub_impl(sum, row);
    }

    #[target_feature(enable = "avx2")]
    unsafe fn dot(weights: &[i16; HIDDEN_SIZE], sums: &[i32; HIDDEN_SIZE]) -> i32 {
        dot_impl(weights, sums)
    }

    pub(super) fn kernels() -> Option<Kernels> {
        if !is_x86_feature_detected!("avx2") {
            return None;
        }
        // SAFETY: the feature check above guarantees the CPU executes AVX2;
        // the functions have no other requirements.
        Some(Kernels {
            add: |sum, row| unsafe { add(sum, row) },
            sub: |sum, row| unsafe { sub(sum, row) },
            dot: |weights, sums| unsafe { dot(weights, sums) },
        })
    }
}

fn select() -> Kernels {
    #[cfg(target_arch = "x86_64")]
    if let Some(kernels) = avx2::kernels() {
        return kernels;
    }
    Kernels {
        add: add_generic,
        sub: sub_generic,
        dot: dot_generic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_agrees_with_the_scalar_definition() {
        let weights: [i16; HIDDEN_SIZE] = std::array::from_fn(|unit| (unit as i16 * 37) - 2000);
        let mut sums: [i32; HIDDEN_SIZE] = std::array::from_fn(|unit| (unit as i32 * 13) - 600);
        let selected = kernels();
        let mut expected = sums;
        add_impl(&mut expected, &weights);
        (selected.add)(&mut sums, &weights);
        assert_eq!(sums, expected);
        sub_impl(&mut expected, &weights);
        (selected.sub)(&mut sums, &weights);
        assert_eq!(sums, expected);
        assert_eq!((selected.dot)(&weights, &sums), dot_impl(&weights, &sums));
    }
}
