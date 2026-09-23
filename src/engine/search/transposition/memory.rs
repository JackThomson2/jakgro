//! Backing memory for the bucket array.
//!
//! The table is the one structure whose page size matters to the search: a
//! probe is one dependent load into memory far larger than any cache, and on
//! 4 KiB pages a table of a few hundred mebibytes misses the translation cache
//! on nearly every probe. The buckets therefore live in a mapping the table
//! owns rather than in whatever the global allocator hands out, aligned to a
//! 2 MiB page and asked to be backed by such pages where the kernel offers
//! them: Linux is advised with `MADV_HUGEPAGE`, and x86-64 macOS is asked for
//! superpages outright. Every request is best effort. A refusal leaves an
//! ordinary mapping, and a platform without mappings uses the global
//! allocator, so the table works the same everywhere and is merely slower
//! where large pages are unavailable.
//!
//! Zero bytes are what an empty slot stores, so a fresh mapping needs no
//! initialisation pass: the kernel commits pages as the search first touches
//! them, and a `Hash` change costs a system call rather than a write of the
//! whole table.

use std::alloc::{self, Layout};
use std::mem::{align_of, size_of};
use std::ptr::NonNull;

use super::{AllocationError, Bucket};

/// The page size large-page requests are made in, and the mapping alignment.
///
/// Both platforms that grant large pages to a process grant 2 MiB ones on the
/// architectures this engine runs on, and a mapping that starts and ends on a
/// 2 MiB boundary can be backed by them throughout.
pub(super) const HUGE_PAGE_BYTES: usize = 2 * 1024 * 1024;

/// A zeroed array of buckets in memory the table owns.
#[derive(Debug)]
pub(super) struct BucketMemory {
    buckets: NonNull<Bucket>,
    count: usize,
    backing: Backing,
}

/// Where the buckets came from, which decides how they are released.
#[derive(Debug)]
enum Backing {
    /// An anonymous mapping of `bytes` starting at the bucket pointer.
    #[cfg(unix)]
    Mapped { bytes: usize },
    /// A block from the global allocator with this layout.
    Heap { layout: Layout },
}

// SAFETY: the memory is owned exclusively by this value until it is dropped,
// and a bucket is made of atomics, so sharing the array between threads is
// exactly what `&[Bucket]` already permits; only the raw pointer defeats the
// automatic judgement.
unsafe impl Send for BucketMemory {}
unsafe impl Sync for BucketMemory {}

impl BucketMemory {
    /// Reserves `count` zeroed buckets.
    ///
    /// Prefers a mapping the kernel can back with large pages and falls back
    /// to the global allocator; fails only when neither can provide the memory.
    pub(super) fn zeroed(count: usize) -> Result<Self, AllocationError> {
        let bytes = count
            .checked_mul(size_of::<Bucket>())
            .filter(|&bytes| bytes > 0)
            .ok_or(AllocationError)?;
        #[cfg(unix)]
        if let Some(memory) = Self::mapped(count, bytes) {
            return Ok(memory);
        }
        Self::heap(count, bytes)
    }

    /// The buckets, every one of them a valid, initially empty bucket.
    #[inline(always)]
    pub(super) fn buckets(&self) -> &[Bucket] {
        // SAFETY: the pointer is aligned for `Bucket` and addresses `count`
        // buckets of memory that this value owns for as long as it lives. The
        // memory was zero-filled, and every field of a bucket is an atomic
        // integer, for which all-zero bytes are a valid value.
        unsafe { std::slice::from_raw_parts(self.buckets.as_ptr(), self.count) }
    }

    fn heap(count: usize, bytes: usize) -> Result<Self, AllocationError> {
        let layout =
            Layout::from_size_align(bytes, align_of::<Bucket>()).map_err(|_| AllocationError)?;
        // SAFETY: the layout has a non-zero size.
        let pointer = unsafe { alloc::alloc_zeroed(layout) };
        let buckets = NonNull::new(pointer.cast::<Bucket>()).ok_or(AllocationError)?;
        Ok(Self {
            buckets,
            count,
            backing: Backing::Heap { layout },
        })
    }

    #[cfg(unix)]
    fn mapped(count: usize, bytes: usize) -> Option<Self> {
        // Whole large pages, so that the tail of the table can be one too.
        let mapped = bytes.checked_next_multiple_of(HUGE_PAGE_BYTES)?;
        let buckets = mapping::large_page_aligned(mapped)?.cast::<Bucket>();
        Some(Self {
            buckets,
            count,
            backing: Backing::Mapped { bytes: mapped },
        })
    }
}

impl Drop for BucketMemory {
    fn drop(&mut self) {
        match self.backing {
            #[cfg(unix)]
            Backing::Mapped { bytes } => {
                // SAFETY: this value mapped exactly this range and nothing
                // else refers to it once the value is dropped.
                unsafe { mapping::release(self.buckets.cast(), bytes) };
            }
            Backing::Heap { layout } => {
                // SAFETY: the pointer came from `alloc_zeroed` with this layout.
                unsafe { alloc::dealloc(self.buckets.as_ptr().cast(), layout) };
            }
        }
    }
}

/// Anonymous mappings aligned to a large page, with the platform's request
/// for large-page backing.
#[cfg(unix)]
mod mapping {
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    use super::HUGE_PAGE_BYTES;

    /// Maps `bytes` of zeroed memory starting on a large-page boundary.
    ///
    /// `bytes` must be a multiple of [`HUGE_PAGE_BYTES`]. Returns `None` when
    /// the kernel refuses every mapping, never because it refused large pages.
    pub(super) fn large_page_aligned(bytes: usize) -> Option<NonNull<c_void>> {
        debug_assert_eq!(bytes % HUGE_PAGE_BYTES, 0);
        if let Some(superpages) = superpages(bytes) {
            return Some(superpages);
        }
        // Map one page more than needed, then unmap the head and tail that
        // fall outside the aligned range; the kernel is free to hand out any
        // page-aligned address, so alignment has to be carved out this way.
        let total = bytes.checked_add(HUGE_PAGE_BYTES)?;
        let base = anonymous(total, 0)?;
        let start = base.as_ptr() as usize;
        let aligned = start.next_multiple_of(HUGE_PAGE_BYTES);
        let head = aligned - start;
        let tail = total - head - bytes;
        // SAFETY: both ranges lie inside the mapping just created, are page
        // aligned, and nothing refers to them.
        unsafe {
            if head > 0 {
                release(base, head);
            }
            if tail > 0 {
                release(
                    NonNull::new_unchecked((aligned + bytes) as *mut c_void),
                    tail,
                );
            }
        }
        // SAFETY: `aligned` lies inside the mapping, which is not null.
        let pointer = unsafe { NonNull::new_unchecked(aligned as *mut c_void) };
        advise_huge_pages(pointer, bytes);
        Some(pointer)
    }

    /// Unmaps `bytes` at `pointer`.
    ///
    /// # Safety
    ///
    /// The range must be part of a live mapping made by this module, and
    /// nothing may refer to it afterwards.
    pub(super) unsafe fn release(pointer: NonNull<c_void>, bytes: usize) {
        // The only failure modes are an invalid range or one that was never
        // mapped, both of which the caller's contract excludes, so the result
        // carries no information worth acting on.
        let _ = unsafe { libc::munmap(pointer.as_ptr(), bytes) };
    }

    /// Maps `bytes` of zeroed private memory, passing `fd` through to the
    /// kernel as the mapping's extra request.
    fn anonymous(bytes: usize, fd: libc::c_int) -> Option<NonNull<c_void>> {
        // SAFETY: an anonymous private mapping with a null hint reads no
        // memory and leaves the kernel to choose the address.
        let pointer = unsafe {
            libc::mmap(
                ptr::null_mut(),
                bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                fd,
                0,
            )
        };
        (pointer != libc::MAP_FAILED)
            .then(|| NonNull::new(pointer))
            .flatten()
    }

    /// Asks x86-64 macOS for a mapping backed by 2 MiB superpages.
    ///
    /// The kernel accepts the request only on that architecture: on Apple
    /// silicon it rejects every superpage size for a process, whose 16 KiB
    /// base pages already cover four times what 4 KiB ones do. The request
    /// travels in the descriptor argument of an anonymous mapping and yields
    /// memory aligned to the superpage, or fails when none are available.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    fn superpages(bytes: usize) -> Option<NonNull<c_void>> {
        let pointer = anonymous(bytes, libc::VM_FLAGS_SUPERPAGE_SIZE_2MB)?;
        if pointer.as_ptr() as usize % HUGE_PAGE_BYTES == 0 {
            return Some(pointer);
        }
        // SAFETY: the mapping was just created and is referred to by nothing.
        unsafe { release(pointer, bytes) };
        None
    }

    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    fn superpages(_bytes: usize) -> Option<NonNull<c_void>> {
        None
    }

    /// Advises Linux to back the range with transparent huge pages.
    ///
    /// Whether it does depends on the system's policy; under the common
    /// `madvise` setting this call is what makes the table eligible at all,
    /// and under `never` it is harmless. The result is deliberately ignored.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn advise_huge_pages(pointer: NonNull<c_void>, bytes: usize) {
        // SAFETY: the range is a live mapping owned by the caller; advice
        // changes no memory and cannot fault.
        let _ = unsafe { libc::madvise(pointer.as_ptr(), bytes, libc::MADV_HUGEPAGE) };
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn advise_huge_pages(_pointer: NonNull<c_void>, _bytes: usize) {}
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};
    use std::sync::atomic::Ordering;

    use super::{BucketMemory, HUGE_PAGE_BYTES};
    use crate::engine::search::transposition::Bucket;

    #[test]
    fn memory_is_zeroed_and_addressable() {
        let memory = BucketMemory::zeroed(3 * 1024).unwrap();
        let buckets = memory.buckets();

        assert_eq!(buckets.len(), 3 * 1024);
        assert_eq!(buckets.as_ptr() as usize % align_of::<Bucket>(), 0);
        for bucket in buckets {
            for slot in &bucket.0 {
                assert_eq!(slot.load(), (0, 0));
            }
        }
    }

    #[test]
    fn nothing_is_not_a_table() {
        assert!(BucketMemory::zeroed(0).is_err());
    }

    /// The buckets must survive being written to and read back, and so must
    /// a mapping whose size is not a whole number of large pages.
    #[test]
    fn written_buckets_read_back() {
        let count = 1024 * 1024 / size_of::<Bucket>() + 7;
        let memory = BucketMemory::zeroed(count).unwrap();
        let buckets = memory.buckets();
        for (index, bucket) in buckets.iter().enumerate() {
            bucket.0[index % 4]
                .data
                .store(index as u64 + 1, Ordering::Relaxed);
        }
        for (index, bucket) in buckets.iter().enumerate() {
            assert_eq!(
                bucket.0[index % 4].data.load(Ordering::Relaxed),
                index as u64 + 1
            );
        }
    }

    /// A mapping starts on a large-page boundary so that it can be backed by
    /// large pages from its first byte.
    #[cfg(unix)]
    #[test]
    fn mappings_are_aligned_to_a_large_page() {
        let memory = BucketMemory::zeroed(HUGE_PAGE_BYTES / size_of::<Bucket>()).unwrap();
        assert!(matches!(memory.backing, super::Backing::Mapped { .. }));
        assert_eq!(memory.buckets().as_ptr() as usize % HUGE_PAGE_BYTES, 0);
    }

    /// Linux records the advice on the mapping, which is what makes it
    /// eligible for transparent huge pages under the `madvise` policy.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mappings_carry_the_huge_page_advice() {
        let memory = BucketMemory::zeroed(2 * HUGE_PAGE_BYTES / size_of::<Bucket>()).unwrap();
        let start = memory.buckets().as_ptr() as usize;
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
        let mut lines = smaps.lines();
        let mut flags = None;
        while let Some(line) = lines.next() {
            let Some((range, _)) = line.split_once(' ') else {
                continue;
            };
            let Some((low, high)) = range.split_once('-') else {
                continue;
            };
            let (Ok(low), Ok(high)) = (
                usize::from_str_radix(low, 16),
                usize::from_str_radix(high, 16),
            ) else {
                continue;
            };
            if low <= start && start < high {
                assert_eq!(low % HUGE_PAGE_BYTES, 0, "mapping {range} is not aligned");
                assert!(
                    high - low >= 2 * HUGE_PAGE_BYTES,
                    "mapping {range} is too small"
                );
                flags = lines.find_map(|line| line.strip_prefix("VmFlags:"));
                break;
            }
        }
        let flags = flags.expect("the table's mapping appears in smaps");
        if std::path::Path::new("/sys/kernel/mm/transparent_hugepage").exists() {
            assert!(
                flags.split_whitespace().any(|flag| flag == "hg"),
                "the mapping is not advised for huge pages: {flags}",
            );
        }
    }
}
