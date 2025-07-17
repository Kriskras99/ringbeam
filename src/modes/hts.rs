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
use std::num::NonZeroU32;

/// A multithreaded consumer or producer that only allows one access at a time.
#[derive(Default)]
pub struct HeadTailSync {
    inner: AtomicU64,
}

#[derive(Copy, Clone)]
struct HeadTail {
    head: u32,
    tail: u32,
}
impl From<u64> for HeadTail {
    fn from(value: u64) -> Self {
        let head = (value >> 32) as u32;
        let tail = (value & 0xFFFF_FFFF) as u32;
        Self { head, tail }
    }
}
impl From<HeadTail> for u64 {
    #[expect(
        clippy::use_self,
        reason = "It's clearer with the explicit integer type"
    )]
    fn from(value: HeadTail) -> Self {
        ((value.head as u64) << 32) | value.tail as u64
    }
}

impl HeadTailSync {
    fn load(&self, order: Ordering) -> HeadTail {
        self.inner.load(order).into()
    }

    fn store(&self, val: HeadTail, order: Ordering) {
        self.inner.store(val.into(), order);
    }

    fn compare_exchange_weak(
        &self,
        current: HeadTail,
        new: HeadTail,
        success: Ordering,
        failure: Ordering,
    ) -> Result<HeadTail, HeadTail> {
        self.inner
            .compare_exchange_weak(current.into(), new.into(), success, failure)
            .map(HeadTail::from)
            .map_err(HeadTail::from)
    }
}

impl ModeInner for HeadTailSync {
    fn move_head<const N: usize, const IS_PROD: bool, Q: QueueBehaviour, Other: Mode>(
        &self,
        other: &Other,
        n: NonZeroU32,
    ) -> Result<Claim, Error> {
        // Get the current head
        let mut old = self.load(Acquire);

        loop {
            while old.head != old.tail {
                spin_loop();
                old = self.load(Acquire);
            }

            let other_tail = other.load_tail(Relaxed);

            let n = calculate_available::<N, IS_PROD, Q>(old.head, other_tail, n)?;

            let new = HeadTail {
                head: old.head.wrapping_add(n.get()) & (N as u32 - 1),
                tail: old.tail,
            };

            match self.compare_exchange_weak(old, new, Acquire, Acquire) {
                Ok(_) => return Ok(Claim::many(n, old.tail)),
                Err(new_old) => {
                    cold_path();
                    old = new_old;
                }
            }
        }
    }

    fn update_tail<const N: usize>(&self, claim: Claim) {
        let new_tail = claim.new_tail::<N>();
        let new = HeadTail {
            head: new_tail,
            tail: new_tail,
        };
        self.store(new, Release);
    }

    fn load_tail(&self, ordering: Ordering) -> u32 {
        self.load(ordering).tail
    }
    fn mark_finished(&self) {
        let res = self.inner.fetch_or(0x8000_0000, Relaxed);
        assert_eq!(res & 0x8000_0000, 0, "Tail was already marked as finished!");
    }
    fn is_finished(&self) -> bool {
        self.inner.load(Relaxed) & 0x8000_0000 != 0
    }
}
