use crate::{
    Error,
    modes::{FixedQueue, Mode, VariableQueue},
    ring::{Ring, active::Last},
    std::hint::cold_path,
};
use std::thread::panicking;

pub struct Sender<const N: usize, T, P, C>
where
    P: Mode,
    C: Mode,
{
    ring: *const Ring<N, T, P, C>,
}

impl<const N: usize, T, P, C> Sender<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// Create a new sender.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C>) -> Self {
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
    pub(crate) unsafe fn new_no_register(ring: *const Ring<N, T, P, C>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            cold_path();
            debug_assert!((*ring).active_producers() == 1);
        }
        Self { ring }
    }

    /// Try to put the value in the channel.
    ///
    /// # Errors
    /// Returns [`Ok(Some(T))`] when full, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    pub fn try_send(&self, value: T) -> Result<Option<T>, Error> {
        let mut once = std::iter::once(value);
        match self.try_send_bulk(&mut once) {
            Ok(1) => Ok(None),
            Err(Error::Full) => {
                cold_path();
                Ok(once.next())
            }
            Err(error) => {
                cold_path();
                Err(error)
            }
            Ok(_) => unreachable!(),
        }
    }

    /// Try to put all values into the channel or none at all.
    ///
    /// To put as many values in the channel as possible, see [`try_send_burst`](Self::try_send_burst).
    ///
    /// # Returns
    /// The amount of values written.
    ///
    /// # Errors
    /// Returns [`Error::Full`] when full, [`Error::NotEnoughSpace`] if there is space but not
    /// enough for all items, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    // TODO: The Iterator must be TrustedLen, but that's unstable
    pub fn try_send_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
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
    /// To return an error when there is not enough space for all the values, see [`try_send_bulk`](Self::try_send_bulk)
    ///
    /// # Returns
    /// The amount of values written.
    ///
    /// # Errors
    /// Returns [`Error::Full`] when full, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    // TODO: The Iterator must be TrustedLen, but that's unstable
    pub fn try_send_burst<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_enqueue::<I, VariableQueue>(values)
    }
}

impl<const N: usize, T, P, C> Clone for Sender<N, T, P, C>
where
    P: Mode + Sync,
    C: Mode,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}

impl<const N: usize, T, P, C> Drop for Sender<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    fn drop(&mut self) {
        if panicking() {
            unsafe {
                // SAFETY: Ring is valid before we call unregister_producer
                (*self.ring).poison();
            }
        } else {
            // SAFETY: Ring is valid before we call unregister_producer
            match unsafe { (*self.ring).unregister_producer().unwrap() } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    // wait for the tail being marked.
                    unsafe {
                        (*self.ring).mark_prod_finished();
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
unsafe impl<const N: usize, T, P, C> Send for Sender<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
}

// SAFETY: Mutable access to the producer head is guarded by atomics, but only for `Multi`.
unsafe impl<const N: usize, T, P, C> Sync for Sender<N, T, P, C>
where
    P: Mode + Sync,
    C: Mode,
{
}
