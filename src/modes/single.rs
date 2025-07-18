//! Implementation for a single-threaded consumer or producer.

use crate::{
    Error,
    modes::{Claim, Mode, ModeInner, QueueBehaviour, calculate_available},
    std::sync::atomic::{
        AtomicU32, Ordering,
        Ordering::{Acquire, Relaxed, Release},
        fence,
    },
};
use core::{marker::PhantomData, num::NonZeroU32};

/// A single threaded consumer or producer.
#[derive(Default)]
pub struct Single {
    /// The current head.
    ///
    /// This is an atomic because all the operations in `Mode` take an immutable reference,
    /// so we need the interior mutability of the atomic type.
    head: AtomicU32,
    /// The current tail.
    ///
    /// This is an atomic because it's used by the other headtail for synchronisation.
    tail: AtomicU32,
    /// `Single` must absolutely not be shared.
    _not_sync: PhantomData<*mut ()>,
}

impl ModeInner for Single {
    fn move_head<const N: usize, const IS_PROD: bool, Q: QueueBehaviour, Other: Mode>(
        &self,
        other: &Other,
        expected: NonZeroU32,
    ) -> Result<Claim, Error> {
        // Get the current head
        let old_head = self.head.load(Relaxed);

        // Ensure head is read before tail (github.com/DPDK/dpdk/commit/86757c2)
        // This works because the compiler/processor is not allowed to reorder operations
        // past two atomic operations.
        fence(Acquire);

        // Sync with update_tail Release (github.com/DPDK/dpdk/commit/9ed8770)
        let other_tail = other.load_tail(Acquire);

        let available = calculate_available::<N, IS_PROD, Q>(old_head, other_tail, expected)?;

        let new_head = old_head.wrapping_add(available.get()) & (N as u32 - 1);

        self.head.store(new_head, Relaxed);
        Ok(Claim::many(available, old_head))
    }

    #[inline]
    fn update_tail<const N: usize>(&self, claim: Claim) {
        let new_tail = claim.new_tail::<N>();
        self.tail.store(new_tail, Release);
    }

    #[inline]
    fn load_tail(&self, ordering: Ordering) -> u32 {
        // TODO: Maybe this can always be Relaxed for Single?
        self.tail.load(ordering)
    }

    #[inline]
    fn mark_finished(&self) {
        let res = self.tail.fetch_or(0x8000_0000, Relaxed);
        assert_eq!(res & 0x8000_0000, 0, "Tail was already marked as finished!");
    }

    #[inline]
    fn is_finished(&self) -> bool {
        self.tail.load(Relaxed) & 0x8000_0000 != 0
    }
}
