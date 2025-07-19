//! The core logic of the ring.
pub mod active;
pub mod recv_values;

use crate::{
    Error,
    cache_padded::CachePadded,
    consumer::Receiver,
    modes::{Claim, Mode},
    producer::Sender,
    ring::{active::AtomicActive, recv_values::RecvValues},
    std::{
        alloc::{Layout, alloc, dealloc, handle_alloc_error},
        cell::UnsafeCell,
        hint::{cold_path, spin_loop},
        mem::MaybeUninit,
        sync::atomic::Ordering::SeqCst,
    },
};
use core::{mem::offset_of, num::NonZeroU32, ops::Deref as _};

/// A ring buffer.
///
/// # Generics
/// - `N`, the capacity of the channel. Must be equal to `2.pow(m)-1` where `m >= 1 && m <= 31`.
/// - `T`, the type of messages that will be sent. `size_of::<T>()` must be a multiple of 4.
/// - `P`, the mode of head-tail synchronisation of producers, see [`Mode`].
/// - `C`, the mode of head-tail synchronisation of consumers, see [`Mode`].
pub struct Ring<const N: usize, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// Tracks the active producers and consumers.
    ///
    /// It can also be used to check if the ring is poisoned.
    active: CachePadded<AtomicActive>,
    /// The head and tail of the producers.
    prod_headtail: CachePadded<P>,
    /// The head and tail of the consumers.
    cons_headtail: CachePadded<C>,
    /// The actual data of the ring.
    ///
    /// # Safety
    /// If an index is between the consumer head and producer tail it **must** be initialized.
    /// A [`Claim`] to a range **must** be owned before trying to access any index in that range.
    data: CachePadded<[UnsafeCell<MaybeUninit<T>>; N]>,
}

impl<const N: usize, T, P, C> Ring<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// Create the ring returning a sender and receiver.
    #[expect(
        clippy::new_ret_no_self,
        reason = "This type should only be used through the sender and receiver"
    )]
    pub fn new() -> (Sender<N, T, P, C>, Receiver<N, T, P, C>) {
        // Check input
        const {
            assert!(
                N >= 2 && N.is_power_of_two() && N <= u32::MAX as usize,
                "Requested capacity was not a power of two"
            );
            // Loom's UnsafeCell type is larger, because it tracks (mutable) references.
            #[cfg(not(any(feature = "loom", feature = "shuttle", feature = "safe_maybeuninit")))]
            assert!(
                size_of::<T>() == size_of::<UnsafeCell<MaybeUninit<T>>>(),
                "Missed optimisation"
            );
        }

        // Allocate the ring
        let layout = Layout::new::<Self>();
        // SAFETY: Layout is valid
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            cold_path();
            handle_alloc_error(layout);
        }

        // Initialize the ring
        // SAFETY: Pointer is not null. The allocation is valid and aligned.
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "The pointers are guaranteed aligned by Layout"
        )]
        unsafe {
            ptr.add(offset_of!(Self, active))
                .cast::<CachePadded<AtomicActive>>()
                .write(CachePadded::new(AtomicActive::new(1, 1)));
            ptr.add(offset_of!(Self, prod_headtail))
                .cast::<CachePadded<P>>()
                .write(CachePadded::default());
            ptr.add(offset_of!(Self, cons_headtail))
                .cast::<CachePadded<C>>()
                .write(CachePadded::default());
            ptr.add(offset_of!(Self, data))
                .cast::<CachePadded<[UnsafeCell<MaybeUninit<T>>; N]>>()
                .write(CachePadded::new(core::array::from_fn(|_| {
                    UnsafeCell::new(MaybeUninit::uninit())
                })));
        }

        // The ring is now initialized and valid
        let ring = ptr.cast::<Self>().cast_const();

        // SAFETY: ring has been initialized and correctly aligned. Producer and consumer counter have
        //         been set to one and we only call new_no_register once.
        let (sender, receiver) = unsafe {
            (
                Sender::new_no_register(ring),
                Receiver::new_no_register(ring),
            )
        };
        (sender, receiver)
    }

    /// Deallocate the ring buffer.
    ///
    /// It will wait for both `cons_headtail` and `prod_headtail` to be marked as finished.
    ///
    /// # Safety
    /// The caller *must* be the last with access to the ring and already unregistered (i.e. `self.active == 0`).
    ///
    /// # Panics
    /// Will panic if the ring still has active producers and/or consumers. It will also panic if
    /// the ring is poisoned.
    ///
    pub unsafe fn cleanup(ring: *const Self) {
        // SAFETY: Ring is still valid before we call dealloc
        unsafe {
            assert!(
                (*ring)
                    .active
                    .load(SeqCst)
                    .is_empty()
                    .expect("The ring is poisoned!"),
                "Still active consumers and/or producers"
            );
            // Wait for the tails to be marked as finished. This is needed as one side can see it's
            // the last on its side, and then try to mark the headtail as finished. But between those
            // two operations the otehr side can discover it's the last and start the cleanup of the ring.
            while !(*ring).cons_headtail.is_finished() && !(*ring).prod_headtail.is_finished() {
                spin_loop();
            }
        }

        let layout = Layout::new::<Self>();
        // SAFETY: `ring` is allocated as this function must only be called once, and the layout
        //         is the same.
        unsafe {
            dealloc(ring.cast::<u8>().cast_mut(), layout);
        }
    }

    /// Mark the prod tail as finished.
    ///
    /// # Safety
    /// This *must* only be called by the last producer.
    #[inline]
    pub unsafe fn mark_prod_finished(&self) {
        self.prod_headtail.mark_finished();
    }

    /// Mark the cons tail as finished.
    ///
    /// # Safety
    /// This *must* only be called by the last consumer.
    #[inline]
    pub unsafe fn mark_cons_finished(&self) {
        self.cons_headtail.mark_finished();
    }

    /// Get access to the producer and consumer tracking.
    pub fn active(&self) -> &AtomicActive {
        &self.active
    }

    /// Get a reference to the data part of the ring.
    #[inline]
    fn data(&self) -> &[UnsafeCell<MaybeUninit<T>>; N] {
        self.data.deref()
    }

    /// Try to enqueue `n` items to the ring.
    ///
    /// If `EXACT` the enqueue will fail if there isn't room for at least `n` entries, otherwise it
    /// can enqueue less than `n` items, leaving the remainder of the items in the iterator.
    ///
    /// # Errors
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. If `EXACT` it
    /// can also return [`Error::NotEnoughSpace`], which can also be successful on a retry.
    pub fn try_enqueue<const EXACT: bool, I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        let Some(len) = NonZeroU32::new(values.len() as u32) else {
            cold_path();
            return Ok(0);
        };

        let claim = self
            .prod_headtail
            .move_head::<N, true, EXACT, _>(self.cons_headtail.deref(), len)
            .map_err(|err| {
                cold_path();
                if err == Error::Closed {
                    cold_path();
                    if self.active.is_poisoned() {
                        Error::Poisoned
                    } else {
                        Error::Closed
                    }
                } else {
                    err
                }
            })?;

        let data = self.data();
        for (i, value) in values.take(claim.entries() as usize).enumerate() {
            let offset = i.wrapping_add(claim.start() as usize) & (N - 1);
            // SAFETY: Our Claim gives exclusive access to this index
            unsafe {
                data[offset].with_mut(|p| (*p).write(value));
            }
        }

        let n = claim.entries() as usize;

        self.prod_headtail.update_tail::<N>(claim);

        Ok(n)
    }

    /// Try to dequeue `n` items from the ring.
    ///
    /// If `EXACT` the dequeue will fail if there aren't at least `n` entries, otherwise it can
    /// return less than `n` items.
    ///
    /// # Errors
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. If `EXACT` it
    /// can also return [`Error::NotEnoughItems`], which can also be successful on a retry. It can
    /// also return [`Error::NotEnoughItemsAndClosed`] where retrying can be successful with `EXACT: false`.
    ///
    /// If there are `u16::MAX - 1` consumers it can also return [`Error::TooManyConsumers`].
    pub fn try_dequeue<const EXACT: bool>(&self, n: usize) -> Result<RecvValues<N, T, P, C>, Error> {
        let Some(len) = NonZeroU32::new(n as u32) else {
            cold_path();
            return Ok(RecvValues::new_empty());
        };
        let claim = self
            .cons_headtail
            .move_head::<N, false, EXACT, _>(self.prod_headtail.deref(), len)
            .map_err(|err| {
                cold_path();
                if err == Error::Closed {
                    cold_path();
                    if self.active.is_poisoned() {
                        Error::Poisoned
                    } else {
                        Error::Closed
                    }
                } else {
                    err
                }
            })?;

        // SAFETY: The ring is valid
        unsafe { RecvValues::new(self, claim) }
    }

    /// Used by [`RecvValues`] to return its [`Claim`].
    #[inline]
    pub fn return_claim_cons(&self, claim: Claim) {
        self.cons_headtail.update_tail::<N>(claim);
    }

    /// Poison the ring.
    ///
    /// After calling this function every function will return [`Error::Poisoned`] or panic.
    ///
    /// This **should** be called if a [`Consumer`], [`Producer`], or [`RecvValues`] panics while holding
    /// a [`Claim`]. Otherwise, the ring will be stuck.
    #[inline]
    pub fn poison(&self) {
        self.active.poison();
        self.cons_headtail.mark_finished();
        self.prod_headtail.mark_finished();
    }
}
