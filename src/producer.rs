use crate::ring::{FixedQueue, IsMulti, Ring, VariableQueue};
use crate::{Error, HeadTail, Multi};
use std::marker::PhantomData;

pub struct Sender<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    ring: *const Ring<N, T, P, C, S, R>,
}

impl<const N: usize, T, P, C, S, R> Sender<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    /// Create a new sender.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C, S, R>) -> Self {
        if !S::IS_MULTI {
            // As only 1 Sender<Single> is allowed to exist this would require ring.active_producers
            // to be zero, but that would mean the channel is closed.
            panic!("Sender<Single> cannot be created through Sender::new");
        }
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_producer().unwrap();
        }
        Self { ring }
    }

    /// Create a new sender but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active senders counter must have already been incremented.
    pub(crate) unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, R>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_producers() == 1);
        }
        Self { ring }
    }

    /// Try to put the value in the channel.
    ///
    /// # Returns
    /// If successful, `Ok(None)`. If full, `Ok(Some(T))`. Otherwise,
    /// it returns `Err(Error::Closed)` or `Err(Error::Poisoned)`.
    fn try_send(&self, value: T) -> Result<Option<T>, Error> {
        let mut once = std::iter::once(value);
        match self.try_send_bulk(&mut once) {
            Ok(1) => Ok(None),
            Err(Error::Closed) => Err(Error::Closed),
            Err(Error::Full) => Ok(once.next()),
            _ => unreachable!(),
        }
    }

    /// Try to put all values into the channel or none at all.
    ///
    /// # Returns
    /// The amount of values written
    // TODO: The Iterator must be TrustedLen, but that's unstable
    fn try_send_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_enqueue::<I, FixedQueue>(values)
    }

    /// Try to put as many values as possible into the channel.
    ///
    /// The implementation will only consume as many values as it can fit into the channel.
    ///
    /// # Returns
    /// The amount of values written
    // TODO: The Iterator must be TrustedLen, but that's unstable
    fn try_send_burst<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_enqueue::<I, VariableQueue>(values)
    }
}

impl<const N: usize, T, P, C, R> Clone for Sender<N, T, P, C, Multi, R>
where
    P: HeadTail,
    C: HeadTail,
    R: IsMulti,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}

impl<const N: usize, T, P, C, S, R> Drop for Sender<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    fn drop(&mut self) {
        // TODO: Poison the ring if panicking
        unsafe {
            if (*self.ring).unregister_producer() {
                // TODO: unallocate the ring
            }
        }
    }
}

// SAFETY: The ring is designed to be accessed from different threads.
unsafe impl<const N: usize, T, P, C, S, R> Send for Sender<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
}

// SAFETY: Mutable access to the producer head is guarded by atomics, but only for `Multi`.
unsafe impl<const N: usize, T, P, C, R> Sync for Sender<N, T, P, C, Multi, R>
where
    P: HeadTail,
    C: HeadTail,
    R: IsMulti,
{
}
