use std::sync::atomic::{AtomicU32, Ordering};

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
            std::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
}
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
            std::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
}

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
            std::hint::spin_loop();
        }
    }

    fn store_tail(&self, tail: u32, order: Ordering) {
        self.tail.store(tail, order);
    }
}
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
}

pub trait HeadTail: HeadTailInner {}
impl<T: HeadTailInner> HeadTail for T {}
