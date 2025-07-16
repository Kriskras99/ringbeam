pub mod active;
pub mod recv_values;

use crate::{
    Error,
    cache_padded::CachePadded,
    consumer::Receiver,
    modes::{Claim, Mode},
    producer::Sender,
    ring::{
        active::{AtomicActive, Last},
        recv_values::RecvValues,
    },
    std::{
        alloc::{Layout, alloc, dealloc, handle_alloc_error},
        cell::UnsafeCell,
        hint::{cold_path, spin_loop},
        mem::MaybeUninit,
        sync::atomic::Ordering::SeqCst,
    },
};
use std::{marker::PhantomData, mem::offset_of, num::NonZeroU32, ops::Deref};

/// A ringbuffer.
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
    phantom: PhantomData<T>,
    /// Active producers and consumers.
    ///
    /// Where the first u16 is the producers and the second u16 is the consumers (`0xPPPP_CCCC`).
    active: CachePadded<AtomicActive>,
    prod_headtail: CachePadded<P>,
    cons_headtail: CachePadded<C>,
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
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            cold_path();
            handle_alloc_error(layout);
        }

        // Initialize the ring
        // SAFETY: this is the only pointer to the data, no references exist
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
                .write(CachePadded::new(std::array::from_fn(|_| {
                    UnsafeCell::new(MaybeUninit::uninit())
                })));
            // phantom is a ZST and can not be initialized (and doesn't need to be either)
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

    /// Deallocate the ringbuffer.
    ///
    /// It will wait for both `cons_headtail` and `prod_headtail` to be marked as finished.
    ///
    /// # Safety
    /// The caller *must* be the last with access to the ring and already unregistered (i.e. `self.active == 0`).
    pub unsafe fn cleanup(ring: *const Self) {
        // SAFETY: Ring is still valid before we call dealloc
        unsafe {
            assert!(
                (*ring).active.load(SeqCst).is_empty(),
                "Still active consumers and/or producers"
            );
            // Wait for the tails to be marked as finished. This is needed as one side can see it's
            // the last on its side, and then try to mark the headtail as finished. But between those
            // two operations the otehr side can discover it's the last and start the cleanup of the ring.
            while !(*ring).cons_headtail.is_finished() && !(*ring).prod_headtail.is_finished() {
                spin_loop();
            }
        };

        let layout = Layout::new::<Self>();
        unsafe { dealloc(ring.cast::<u8>().cast_mut(), layout) };
    }

    /// Mark the prod tail as finished.
    ///
    /// # Safety
    /// This *must* only be called by the last producer.
    pub unsafe fn mark_prod_finished(&self) {
        self.prod_headtail.mark_finished();
    }

    /// Mark the cons tail as finished.
    ///
    /// # Safety
    /// This *must* only be called by the last consumer.
    pub unsafe fn mark_cons_finished(&self) {
        self.cons_headtail.mark_finished();
    }

    /// Get a reference to the data part of the ring.
    fn data(&self) -> &[UnsafeCell<MaybeUninit<T>>; N] {
        self.data.deref()
    }

    pub fn try_enqueue<I, Q>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
        Q: crate::modes::QueueBehaviour,
    {
        let Some(len) = NonZeroU32::new(values.len() as u32) else {
            cold_path();
            return Ok(0);
        };
        let claim = match self
            .prod_headtail
            .move_head::<N, true, Q, _>(self.cons_headtail.deref(), len)
        {
            Ok(claim) => claim,
            Err(err) => {
                cold_path();
                return Err(err);
            }
        };

        let data = self.data();
        for (i, value) in values.take(claim.entries() as usize).enumerate() {
            let mut offset = i + claim.start() as usize;
            if offset >= N {
                cold_path();
                offset -= N;
            }
            // SAFETY: Our Claim gives exclusive access to this index
            unsafe {
                data[offset].with_mut(|p| (&mut *p).write(value));
            }
        }

        let n = claim.entries() as usize;

        self.prod_headtail.update_tail::<N>(claim);

        Ok(n)
    }

    pub fn try_dequeue<Q>(&self, n: usize) -> Result<RecvValues<N, T, P, C>, Error>
    where
        Q: crate::modes::QueueBehaviour,
    {
        let Some(len) = NonZeroU32::new(n as u32) else {
            cold_path();
            return Ok(RecvValues::new_empty());
        };
        let claim = match self
            .cons_headtail
            .move_head::<N, false, Q, _>(self.prod_headtail.deref(), len)
        {
            Ok(claim) => claim,
            Err(err) => {
                cold_path();
                return Err(err);
            }
        };

        unsafe { RecvValues::new(self, claim) }
    }

    pub fn return_claim_cons(&self, claim: Claim) {
        self.cons_headtail.update_tail::<N>(claim);
    }

    pub fn poison(&self) {
        // TODO: also poison headtails
        self.active.poison();
    }
}

/// Functions for keeping track of the active consumers and producers.
impl<const N: usize, T, P, C> Ring<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    pub fn register_producer(&self) -> Result<(), Error> {
        self.active.register_producer()
    }
    pub fn register_consumer(&self) -> Result<(), Error> {
        self.active.register_consumer()
    }
    pub fn unregister_producer(&self) -> Result<Last, Error> {
        self.active.unregister_producer()
    }
    pub fn unregister_consumer(&self) -> Result<Last, Error> {
        self.active.unregister_consumer()
    }
    pub fn active_producers(&self) -> u16 {
        self.active.active_producers()
    }
    pub fn active_consumers(&self) -> u16 {
        self.active.active_consumers()
    }
}
