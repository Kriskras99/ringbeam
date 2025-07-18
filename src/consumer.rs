//! The user facing consumer implementation.

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
    /// The actual ring.
    ///
    /// This pointer is valid and aligned for the entire lifetime of [`Receiver`].
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
    ///
    /// # Errors
    /// Will return [`Error::Closed`] or [`Error::Poisoned`], if the ring is in that state. It will
    /// return [`Error::TooManyConsumers`] if there are already `u16::MAX - 1` consumers.
    ///
    /// Note: A [`RecvValues`] instance also counts as a consumer while [`RecvValues::next`] still
    /// returns `Some`.
    #[inline]
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C>) -> Result<Self, Error> {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).active().register_consumer()?;
        }
        Ok(Self { ring })
    }

    /// Create a new receiver but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active consumers counter must have already been incremented.
    #[inline]
    pub(crate) unsafe fn new_no_register(ring: *const Ring<N, T, P, C>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            cold_path();
            debug_assert!(
                (*ring).active().consumers() == Ok(1),
                "This function must only be called when initializing the ring"
            );
        }
        Self { ring }
    }

    /// Try to get one item from the channel.
    ///
    /// # Errors
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. It can also
    /// return [`Error::TooManyConsumers`] if there are already `u16::MAX - 1` instances of `Receiver`s
    /// and [`RecvValues`].
    #[inline]
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
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. It can also
    /// return [`Error::TooManyConsumers`] if there are already `u16::MAX - 1` instances of `Receiver`s
    /// and [`RecvValues`].
    ///
    /// It can also return [`Error::NotEnoughItems`], which can also be successful on
    /// a retry. It can also return [`Error::NotEnoughItemsAndClosed`] indicating that this will
    /// keep failing with `try_recv_bulk` as there won't be new items.
    #[inline]
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
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. It can also
    /// return [`Error::TooManyConsumers`] if there are already `u16::MAX - 1` instances of `Receiver`s
    /// and [`RecvValues`].
    #[inline]
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
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring).expect("Failed to clone consumer!") }
    }
}

impl<const N: usize, T, P, C> Drop for Receiver<N, T, P, C>
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
            // SAFETY: Ring is valid before we call unregister_consumer
            unsafe {
                (*self.ring).poison();
            }
        } else {
            // SAFETY: Ring is valid before we call unregister_consumer
            match unsafe {
                (*self.ring)
                    .active()
                    .unregister_consumer()
                    .expect("Ring is poisoned!")
            } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    // wait for the tail being marked.
                    unsafe {
                        (*self.ring).mark_cons_finished();
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
