use crate::atomic::{AtomicU32, Ordering};

/// The default sync mode for the head and tail.
///
/// In this mode, multiple participants can move the head forward and claim a part of the buffer.
/// To release there claim they need to wait for the tail to move to their claim. If a thread
/// holds on to a claim for a long time, this will cause a wait in all other threads with later
/// claims.
///
/// This mode is designed for the thread-per-core model and is known to behave poorly in over commited
/// scenarios.
#[derive(Default)]
pub struct TailSync {
    head: AtomicU32,
    tail: AtomicU32,
}
impl HeadTailInner for TailSync {
    fn load_head(&self, order: Ordering) -> u32 {
        self.head.load(order)
    }

    fn load_tail(&self, order: Ordering) -> u32 {
        self.tail.load(order)
    }

    fn store_head(&self, head: u32, order: Ordering) {
        self.head.store(head, order);
    }

    fn compare_exchange_weak_head(
        &self,
        current: u32,
        new: u32,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u32, u32> {
        self.head
            .compare_exchange_weak(current, new, success, failure)
    }

    fn wait_until_equal_tail(&self, expected: u32, order: Ordering) {
        while self.tail.load(order) != expected {
            crate::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
    fn mark_finished(&self) {
        let res = self.tail.fetch_or(0x8000_0000, Ordering::SeqCst);
        assert_eq!(res & 0x8000_0000, 0, "Tail was already marked as finished!");
    }
    fn is_finished(&self) -> bool {
        self.tail.load(Ordering::SeqCst) & 0x8000_0000 != 0
    }
}
impl HeadTail for TailSync {}

/// A sync mode for head and tail that does not require participants to wait on each other.
///
/// This mode makes the last thread responsible for updating the tail value, allowing other threads
/// to continue.
///
/// This mode helps to avoid the Lock-Waiter-Preemption (LWP) problem on tail update and improves
/// average enqueue/dequeue times on overcommitted systems.
// TODO: Actually implement this
#[derive(Default)]
pub struct RelaxedTailSync {
    head: AtomicU32,
    tail: AtomicU32,
}
impl HeadTailInner for RelaxedTailSync {
    fn load_head(&self, order: Ordering) -> u32 {
        self.head.load(order)
    }

    fn load_tail(&self, order: Ordering) -> u32 {
        self.tail.load(order)
    }

    fn store_head(&self, head: u32, order: Ordering) {
        self.head.store(head, order);
    }

    fn compare_exchange_weak_head(
        &self,
        current: u32,
        new: u32,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u32, u32> {
        self.head
            .compare_exchange_weak(current, new, success, failure)
    }

    fn wait_until_equal_tail(&self, expected: u32, order: Ordering) {
        while self.tail.load(order) != expected {
            crate::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
    fn mark_finished(&self) {
        let res = self.tail.fetch_or(0x8000_0000, Ordering::SeqCst);
        assert_eq!(res & 0x8000_0000, 0, "Tail was already marked as finished!");
    }
    fn is_finished(&self) -> bool {
        self.tail.load(Ordering::SeqCst) & 0x8000_0000 != 0
    }
}
impl HeadTail for RelaxedTailSync {}

/// A sync mode for head and tail that only allows one enqueue and one dequeue at a time.
///
/// This mode also avoids the Lock-Waiter-Preemption (LWP) problem on tail update and helps to
/// improve ring enqueue/dequeue behavior in overcommitted scenarios.
// TODO: Actually implement this
#[derive(Default)]
pub struct HeadTailSync {
    head: AtomicU32,
    tail: AtomicU32,
}
impl HeadTailInner for HeadTailSync {
    fn load_head(&self, order: Ordering) -> u32 {
        self.head.load(order)
    }

    fn load_tail(&self, order: Ordering) -> u32 {
        self.tail.load(order)
    }

    fn store_head(&self, head: u32, order: Ordering) {
        self.head.store(head, order);
    }

    fn compare_exchange_weak_head(
        &self,
        current: u32,
        new: u32,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u32, u32> {
        self.head
            .compare_exchange_weak(current, new, success, failure)
    }

    fn wait_until_equal_tail(&self, expected: u32, order: Ordering) {
        while self.tail.load(order) != expected {
            crate::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
    fn mark_finished(&self) {
        let res = self.tail.fetch_or(0x8000_0000, Ordering::SeqCst);
        assert_eq!(res & 0x8000_0000, 0, "Tail was already marked as finished!");
    }
    fn is_finished(&self) -> bool {
        self.tail.load(Ordering::SeqCst) & 0x8000_0000 != 0
    }
}
impl HeadTail for HeadTailSync {}

pub trait HeadTailInner: Default {
    /// Load the head with the given order.
    fn load_head(&self, order: Ordering) -> u32;
    /// Load the tail with the given order.
    fn load_tail(&self, order: Ordering) -> u32;
    fn store_head(&self, head: u32, order: Ordering);
    fn compare_exchange_weak_head(
        &self,
        current: u32,
        new: u32,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u32, u32>;
    fn wait_until_equal_tail(&self, expected: u32, order: Ordering);
    fn store_tail(&self, tail: u32, order: Ordering);
    fn mark_finished(&self);
    fn is_finished(&self) -> bool;
}

pub trait HeadTail: HeadTailInner {}
