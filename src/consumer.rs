use crate::ring::{IsMulti, Ring};
use crate::{HeadTail, Multi, Single};
use std::marker::PhantomData;

pub struct ReceiverAct<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, S, R>,
}

impl<const N: usize, T, P, C, S, R> ReceiverAct<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    /// Create a new receiver.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C, S, R>) -> Self {
        if !S::IS_MULTI {
            // As only 1 Receiver<Single> is allowed to exist this would require ring.active_consumers
            // to be zero, but that would mean the channel is closed.
            panic!("Receiver<Single> cannot be created through Receiver::new");
        }
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_consumer().unwrap();
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    /// Create a new receiver but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active consumers counter must have already been incremented.
    pub(crate) unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, R>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }
}

pub trait ReceiverInner<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
    Self: Sized,
{
    /// Create a new receiver.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    unsafe fn new(ring: *const Ring<N, T, P, C, S, R>) -> Self;

    /// Create a new receiver but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active receivers counter must have already been incremented.
    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, R>) -> Self;
}

pub trait Receiver<const N: usize, T, P, C, S, R>: ReceiverInner<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
}
impl<const N: usize, T, P, C, S, R, Inner> Receiver<N, T, P, C, S, R> for Inner
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
    Inner: ReceiverInner<N, T, P, C, S, R>,
{
}

pub struct MultiConsumer<const N: usize, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, S, Multi>,
}

impl<const N: usize, T, P, C, S> ReceiverInner<N, T, P, C, S, Multi>
    for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
    unsafe fn new(ring: *const Ring<N, T, P, C, S, Multi>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_consumer().unwrap();
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, Multi>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }
}

impl<const N: usize, T, P, C, S> Clone for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}

// SAFETY: The ring is designed to be accessed from different threads.
unsafe impl<const N: usize, T, P, C, S> Send for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
}

// SAFETY: Mutable access to the data is guarded by atomics.
unsafe impl<const N: usize, T, P, C, S> Sync for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
}

pub struct SingleConsumer<const N: usize, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, S, Single>,
}
impl<const N: usize, T, P, C, S> ReceiverInner<N, T, P, C, S, Single>
    for SingleConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
    unsafe fn new(ring: *const Ring<N, T, P, C, S, Single>) -> Self {
        // As only 1 SingleConsumer is allowed to exist this would require ring.active_consumers
        // to be zero, but that would mean the channel is closed.
        panic!("SingleConsumer cannot be created through ReceiverInner::new");
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, Single>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }
}

// SAFETY: The ring is designed to be accessed from different threads.
unsafe impl<const N: usize, T, P, C, S> Send for SingleConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
}
