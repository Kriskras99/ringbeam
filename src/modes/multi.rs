//! Implementation for a multithreaded consumer or producer.

use crate::{
    Error,
    modes::{Claim, Mode, ModeInner, QueueBehaviour, calculate_available},
    std::{
        hint::{cold_path, spin_loop},
        sync::atomic::{
            AtomicU32, Ordering,
            Ordering::{Acquire, Relaxed, Release},
            fence,
        },
    },
};
use core::num::NonZeroU32;

/// A multithreaded consumer or producer.
#[derive(Default)]
pub struct Multi {
    /// The current head.
    head: AtomicU32,
    /// The current tail.
    tail: AtomicU32,
}

impl ModeInner for Multi {
    fn move_head<const N: usize, const IS_PROD: bool, Q: QueueBehaviour, Other: Mode>(
        &self,
        other: &Other,
        expected: NonZeroU32,
    ) -> Result<Claim, Error> {
        // Get the current head
        let mut old_head = self.head.load(Relaxed);

        loop {
            // Ensure head is read before tail (github.com/DPDK/dpdk/commit/86757c2)
            // This works because the compiler/processor is not allowed to reorder operations
            // past two atomic operations.
            fence(Acquire);

            // Sync with update_tail Release (github.com/DPDK/dpdk/commit/9ed8770)
            let other_tail = other.load_tail(Acquire);

            let available = calculate_available::<N, IS_PROD, Q>(old_head, other_tail, expected)?;

            let new_head = old_head.wrapping_add(available.get()) & (N as u32 - 1);

            match self
                .head
                .compare_exchange_weak(old_head, new_head, Relaxed, Relaxed)
            {
                Ok(_) => return Ok(Claim::many(available, old_head)),
                Err(new_old_head) => {
                    cold_path();
                    old_head = new_old_head;
                }
            }
        }
    }

    #[inline]
    fn update_tail<const N: usize>(&self, claim: Claim) {
        while self.tail.load(Relaxed) != claim.start {
            // TODO: WFE/SEV optimisation
            spin_loop();
        }
        let new_tail = claim.new_tail::<N>();
        self.tail.store(new_tail, Release);
    }

    #[inline]
    fn load_tail(&self, ordering: Ordering) -> u32 {
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
