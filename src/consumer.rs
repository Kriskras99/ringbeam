use crate::ring::{
    FixedQueue, IsMulti, Ring, VariableQueue, active::Last, recv_values::RecvValues,
};
use crate::{Error, HeadTail, Multi, cold_path};
use std::thread::panicking;

pub struct Receiver<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    ring: *const Ring<N, T, P, C, S, R>,
}

impl<const N: usize, T, P, C, S, R> Receiver<N, T, P, C, S, R>
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
        // As only 1 Receiver<Single> is allowed to exist this would require ring.active_consumers
        // to be zero, but that would mean the channel is closed.
        assert!(
            S::IS_MULTI,
            "Receiver<Single> cannot be created through Receiver::new"
        );

        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_consumer().unwrap();
        }
        Self { ring }
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
            cold_path();
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self { ring }
    }

    /// Try to get one item from the channel.
    pub fn try_recv(&self) -> Result<T, Error> {
        match self.try_recv_bulk(1) {
            Ok(mut res) => {
                let value = res.next().unwrap_or_else(|| unreachable!());
                drop(res);
                Ok(value)
            }
            Err(e) => {
                cold_path();
                Err(e)
            }
        }
    }

    /// Try to get `n` items from the channel or none at all.
    ///
    /// # Returns
    /// An iterator over the items. This iterator is allowed to outlive the receiver.
    /// Dropping the iterator while it still has items, will also drop those items.
    pub fn try_recv_bulk(&self, n: usize) -> Result<RecvValues<N, T, P, C, S, R>, Error> {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_dequeue::<FixedQueue>(n)
    }

    /// Try to get at most `n` items from the channel.
    ///
    /// # Returns
    /// An iterator over the items. This iterator is allowed to outlive the receiver.
    /// Dropping the iterator while it still has items, will also drop those items.
    pub fn try_recv_burst(&self, n: usize) -> Result<RecvValues<N, T, P, C, S, R>, Error> {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_dequeue::<VariableQueue>(n)
    }
}

impl<const N: usize, T, P, C, S> Clone for Receiver<N, T, P, C, S, Multi>
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

impl<const N: usize, T, P, C, S, R> Drop for Receiver<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    fn drop(&mut self) {
        if panicking() {
            unsafe {
                // SAFETY: Ring is valid before we call unregister_consumer
                (*self.ring).poison();
            }
        } else {
            // SAFETY: Ring is valid before we call unregister_consumer
            match unsafe { (*self.ring).unregister_consumer().unwrap() } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    // wait for the tail being marked.
                    unsafe {
                        (*self.ring).mark_cons_finished();
                    }
                }
                Last::InRing => {
                    // Drop the ring as we're last
                    unsafe { Ring::cleanup(self.ring) }
                }
                Last::NotLast => {}
            }
        }
    }
}

// SAFETY: The ring is designed to be accessed from different threads.
unsafe impl<const N: usize, T, P, C, S, R> Send for Receiver<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
}

// SAFETY: Mutable access to the consumer head is guarded by atomics, but only for `Multi`.
unsafe impl<const N: usize, T, P, C, S> Sync for Receiver<N, T, P, C, S, Multi>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
{
}
