//! The user facing producer implementation.

use crate::{
    Error,
    modes::Mode,
    ring::{Ring, active::Last},
    std::hint::cold_path,
};
use std::thread::panicking;

/// The sending-half of the channel.
///
/// # Generics
/// - `N`: the size of the channel.
/// - `T`: the type being sent over the channel.
/// - `P`: the synchronisation mode of the sender, see [`Mode`].
/// - `C`: the synchronisation mode of the receiver, see [`Mode`].
pub struct Sender<const N: usize, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// The actual ring.
    ///
    /// This pointer is valid and aligned for the entire lifetime of [`Sender`].
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
    ///
    /// # Errors
    /// Can return [`Error::Closed`] or [`Error::Poisoned`] when the ring is in that state. It can
    /// return [`Error::TooManyProducers`] if there are already `u16::MAX - 1` producers.
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C>) -> Result<Self, Error> {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).active().register_producer()?;
        }
        Ok(Self { ring })
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
            debug_assert!(
                (*ring).active().producers() == Ok(1),
                "This function must only be called when initializing the ring"
            );
        }
        Self { ring }
    }

    /// Try to put the value in the channel.
    ///
    /// # Errors
    /// Returns [`Ok(Some(T))`] when full, [`Error::Closed`] when closed, and [`Error::Poisoned`]
    /// when the ring is poisoned.
    #[inline]
    pub fn try_send(&self, value: T) -> Result<Option<T>, Error> {
        let mut once = core::iter::once(value);
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
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. It can also
    /// return [`Error::NotEnoughSpace`], which can also be successful on a retry.
    ///
    /// # Panics
    /// Can panic if the [`ExactSizeIterator`] implementation of `I` is wrong.
    #[inline]
    pub fn try_send_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_enqueue::<true, I>(values)
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
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful.
    ///
    /// # Panics
    /// Can panic if the [`ExactSizeIterator`] implementation of `I` is wrong.
    #[inline]
    pub fn try_send_burst<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        // SAFETY: `self` is valid therefore `ring` is initialized and aligned.
        //         No mutable aliasing in the ring except for inside the UnsafeCell.
        let ring = unsafe { &*self.ring };

        ring.try_enqueue::<false, I>(values)
    }
}

impl<const N: usize, T, P, C> Clone for Sender<N, T, P, C>
where
    P: Mode + Sync,
    C: Mode,
{
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring).expect("Failed to clone producer!") }
    }
}

impl<const N: usize, T, P, C> Drop for Sender<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    #[expect(
        clippy::missing_inline_in_public_items,
        reason = "This function is too large too inline"
    )]
    fn drop(&mut self) {
        if panicking() {
            cold_path();
            // SAFETY: Ring is valid before we call unregister_producer
            unsafe {
                (*self.ring).poison();
            }
        } else {
            // SAFETY: Ring is valid before we call unregister_producer
            match unsafe {
                (*self.ring)
                    .active()
                    .unregister_producer()
                    .expect("Ring is poisoned!")
            } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    // wait for the tail being marked.
                    unsafe {
                        (*self.ring).mark_prod_finished();
                    }
                }
                Last::InRing => {
                    // SAFETY: `Last::InRing` guarantees that we're the last
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
