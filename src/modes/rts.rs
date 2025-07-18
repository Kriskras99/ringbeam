//! Implementation for a multithreaded consumer or producer where the tail is updated by the last thread.

use crate::{
    Error,
    modes::{Claim, Mode, ModeInner, QueueBehaviour, calculate_available},
    std::{
        hint::{cold_path, spin_loop},
        sync::atomic::{
            AtomicU64, Ordering,
            Ordering::{Acquire, Relaxed, Release},
        },
    },
};
use core::num::NonZeroU32;

/// A multithreaded consumer or producer where the tail is updated by the last thread.
#[repr(C)]
pub struct RelaxedTailSync {
    /// The current head.
    head: AtomicPosCnt,
    /// Maximum distance between the head and tail.
    htd_max: NonZeroU32,
    /// The current tail.
    tail: AtomicPosCnt,
}

impl Default for RelaxedTailSync {
    #[inline]
    fn default() -> Self {
        Self::new(NonZeroU32::new(5).unwrap_or_else(|| unreachable!()))
    }
}

impl RelaxedTailSync {
    #[must_use]
    #[inline]
    pub fn new(htd_max: NonZeroU32) -> Self {
        Self {
            head: AtomicPosCnt::default(),
            htd_max,
            tail: AtomicPosCnt::default(),
        }
    }
}

/// The head and tail of the head/tail.
#[derive(Copy, Clone, Debug)]
struct PosCnt {
    /// The head of the head/tail
    pos: u32,
    /// The tail of the head/tail
    cnt: u32,
}
impl From<u64> for PosCnt {
    #[inline]
    fn from(value: u64) -> Self {
        let pos = (value >> 32) as u32;
        let cnt = (value & 0xFFFF_FFFF) as u32;
        Self { pos, cnt }
    }
}
impl From<PosCnt> for u64 {
    #[expect(
        clippy::use_self,
        reason = "It's clearer with the explicit integer type"
    )]
    #[inline]
    fn from(value: PosCnt) -> Self {
        ((value.pos as u64) << 32) | value.cnt as u64
    }
}

/// An atomic version of [`PosCnt`] that can be safely shared between threads.
#[derive(Default)]
struct AtomicPosCnt {
    /// The encoded value of [`PosCnt`].
    inner: AtomicU64,
}
impl AtomicPosCnt {
    /// Loads the [`PosCnt`] value atomically.
    ///
    /// See [`AtomicU64::load`].
    #[inline]
    fn load(&self, order: Ordering) -> PosCnt {
        self.inner.load(order).into()
    }

    /// Store a [`PosCnt`] value if the current value is the same as `current`.
    ///
    /// See [`AtomicU64::compare_exchange_weak`].
    #[inline]
    #[expect(clippy::missing_errors_doc, reason = "Not really an error")]
    fn compare_exchange_weak(
        &self,
        current: PosCnt,
        new: PosCnt,
        success: Ordering,
        failure: Ordering,
    ) -> Result<PosCnt, PosCnt> {
        self.inner
            .compare_exchange_weak(current.into(), new.into(), success, failure)
            .map(PosCnt::from)
            .map_err(PosCnt::from)
    }
}

impl ModeInner for RelaxedTailSync {
    fn move_head<const N: usize, const IS_PROD: bool, Q: QueueBehaviour, Other: Mode>(
        &self,
        other: &Other,
        expected: NonZeroU32,
    ) -> Result<Claim, Error> {
        // Get the current head
        let mut old_head = self.head.load(Acquire);

        loop {
            while old_head.pos - old_head.cnt > self.htd_max.get() {
                spin_loop();
                old_head = self.head.load(Acquire);
            }
            // Sync with update_tail Release (github.com/DPDK/dpdk/commit/9ed8770)
            let other_tail = other.load_tail(Acquire);

            let available =
                calculate_available::<N, IS_PROD, Q>(old_head.pos, other_tail, expected)?;

            let new_head = PosCnt {
                pos: old_head.pos.wrapping_add(available.get()) & (N as u32 - 1),
                cnt: old_head.cnt.wrapping_add(1) & (N as u32 - 1),
            };

            match self
                .head
                .compare_exchange_weak(old_head, new_head, Acquire, Acquire)
            {
                Ok(_) => return Ok(Claim::many(available, old_head.pos)),
                Err(new_old_head) => {
                    cold_path();
                    old_head = new_old_head;
                }
            }
        }
    }

    fn update_tail<const N: usize>(&self, claim: Claim) {
        // TODO: This is broken, tail.pos is never updated
        let mut old_tail = self.tail.load(Acquire);
        let _ = claim.new_tail::<N>();
        loop {
            let head = self.head.load(Relaxed);
            let mut new_tail = PosCnt {
                cnt: old_tail.cnt.wrapping_add(1) & (N as u32 - 1),
                pos: old_tail.pos,
            };
            if new_tail.cnt == head.cnt {
                new_tail.pos = head.pos;
            }
            match self
                .tail
                .compare_exchange_weak(old_tail, new_tail, Release, Acquire)
            {
                Ok(_) => return,
                Err(new_old_tail) => {
                    cold_path();
                    old_tail = new_old_tail;
                }
            }
        }
    }

    #[inline]
    fn load_tail(&self, ordering: Ordering) -> u32 {
        self.tail.load(ordering).pos
    }

    #[inline]
    fn mark_finished(&self) {
        let res = self.tail.inner.fetch_or(0x8000_0000_0000_0000, Relaxed);
        assert_eq!(
            res & 0x8000_0000_0000_0000,
            0,
            "Tail was already marked as finished!"
        );
    }

    #[inline]
    fn is_finished(&self) -> bool {
        self.tail.inner.load(Relaxed) & 0x8000_0000_0000_0000 != 0
    }
}
