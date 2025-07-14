use crate::{
    Error,
    modes::{FixedQueue, Mode, VariableQueue},
    ring::{Ring, active::Last, recv_values::RecvValues},
    std::hint::cold_path,
};
use std::thread::panicking;

pub struct Receiver<const N: usize, T, P, C>
where
    P: Mode,
    C: Mode,
{
    ring: *const Ring<N, T, P, C>,
}

impl<const N: usize, T, P, C> Receiver<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// Create a new receiver.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C>) -> Self {
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
    pub(crate) unsafe fn new_no_register(ring: *const Ring<N, T, P, C>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            cold_path();
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self { ring }
    }

    /// Try to get one item from the channel.
    ///
    /// # Errors
    /// Returns [`Error::Empty`] when empty, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
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
    /// To get at most `n` items, see [`try_recv_burst`](Self::try_recv_burst).
    ///
    /// # Returns
    /// An iterator over the items. This iterator is allowed to outlive the receiver.
    /// Dropping the iterator while it still has items, will also drop those items.
    ///
    /// # Errors
    /// Returns [`Error::Empty`] when empty, [`Error::NotEnoughItems`] if there are items but not
    /// as many as requested, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    pub fn try_recv_bulk(&self, n: usize) -> Result<RecvValues<N, T, P, C>, Error> {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_dequeue::<FixedQueue>(n)
    }

    /// Try to get at most `n` items from the channel.
    ///
    /// To get exactly `n` items or none at all, see [`try_recv_bulk`](Self::try_recv_bulk).
    ///
    /// # Returns
    /// An iterator over the items. This iterator is allowed to outlive the receiver.
    /// Dropping the iterator while it still has items, will also drop those items.
    ///
    /// # Errors
    /// Returns [`Error::Empty`] when empty, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    pub fn try_recv_burst(&self, n: usize) -> Result<RecvValues<N, T, P, C>, Error> {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_dequeue::<VariableQueue>(n)
    }
}

impl<const N: usize, T, P, C> Clone for Receiver<N, T, P, C>
where
    P: Mode,
    C: Mode + Sync,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}

impl<const N: usize, T, P, C> Drop for Receiver<N, T, P, C>
where
    P: Mode,
    C: Mode,
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
unsafe impl<const N: usize, T, P, C> Send for Receiver<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
}

// SAFETY: Mutable access to the consumer head is guarded by atomics, but only for those that implement Sync.
unsafe impl<const N: usize, T, P, C> Sync for Receiver<N, T, P, C>
where
    P: Mode,
    C: Mode + Sync,
{
}
