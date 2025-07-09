pub mod active;
pub mod recv_values;

use crate::{
    Error, HeadTail,
    atomic::{
        Ordering::{Acquire, Relaxed, SeqCst},
        fence,
    },
    cache_padded::CachePadded,
    cell::UnsafeCell,
    cold_path,
    consumer::Receiver,
    hint::spin_loop,
    producer::Sender,
    ring::{
        active::{AtomicActive, Last},
        recv_values::RecvValues,
    },
    sealed::Sealed,
};
use std::{
    marker::PhantomData,
    mem::{ManuallyDrop, MaybeUninit, offset_of},
    ops::Deref,
};

pub enum Multi {}
impl Sealed for Multi {}
impl IsMulti for Multi {
    const IS_MULTI: bool = true;
}

pub enum Single {}
impl Sealed for Single {}
impl IsMulti for Single {
    const IS_MULTI: bool = false;
}

/// Can more than instance of the sender/receiver exist?
pub trait IsMulti: Sealed {
    const IS_MULTI: bool;
}

/// A ringbuffer.
///
/// # Generics
/// - `N`, the capacity of the channel. Must be equal to `2.pow(m)-1` where `m >= 1 && m <= 31`.
/// - `T`, the type of messages that will be sent. `size_of::<T>()` must be a multiple of 4.
/// - `P`, the mode of head-tail synchronisation of producers, see [`HeadTail`].
/// - `C`, the mode of head-tail synchronisation of consumers, see [`HeadTail`].
/// - `S`, the type of sender, see [`Sender`].
/// - `R`, the type of receiver, see [`Receiver`].
pub struct Ring<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    phantom: PhantomData<(T, S, R)>,
    /// Active producers and consumers.
    ///
    /// Where the first u16 is the producers and the second u16 is the consumers (`0xPPPP_CCCC`).
    active: CachePadded<AtomicActive>,
    prod_headtail: CachePadded<P>,
    cons_headtail: CachePadded<C>,
    data: CachePadded<[UnsafeCell<MaybeUninit<T>>; N]>,
}

impl<const N: usize, T, P, C, S, R> Ring<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    /// Create the ring returning a sender and receiver.
    #[expect(
        clippy::new_ret_no_self,
        reason = "This type should only be used through the sender and receiver"
    )]
    pub fn new() -> (Sender<N, T, P, C, S, R>, Receiver<N, T, P, C, S, R>) {
        // Check input
        const {
            assert!(
                N >= 2 && N.is_power_of_two() && N <= u32::MAX as usize,
                "Requested capacity was not a power of two"
            );
            assert!(
                size_of::<T>() == size_of::<UnsafeCell<MaybeUninit<T>>>(),
                "Missed optimisation"
            );
        }

        // Allocate the ring
        let layout = crate::alloc::Layout::new::<Self>();
        let ptr = unsafe { crate::alloc::alloc(layout) };
        if ptr.is_null() {
            cold_path();
            std::alloc::handle_alloc_error(layout);
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
            // phantom is a ZST and can not be initialized (and doesn't need to be either)
            // data is a UnsafeCell<[T; N]> and must not be read when uninitialized
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

        let layout = crate::alloc::Layout::new::<Self>();
        unsafe { crate::alloc::dealloc(ring.cast::<u8>().cast_mut(), layout) };
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

    /// Move the producers head to get `n` entries.
    ///
    /// # Returns
    /// The amount of acquired entries, which is smaller or equal to `n` and can be zero.
    fn claim_prod<Q>(&self, n: u32) -> Result<Claim, Error>
    where
        Q: QueueBehaviour,
    {
        let mut old_head = self.prod_headtail.load_head(Relaxed);

        loop {
            // Ensure the head is read before the tail
            fence(Acquire);

            // load-acquire synchronize with store-release of cons_update_tail
            let cons_tail = self.cons_headtail.load_tail(Acquire);

            let entries = Self::uninitialized_entries(old_head, cons_tail);

            if entries == 0 {
                cold_path();
                // Check if the MSB is set, as that indicates the channel is closed on the other side
                if cons_tail & 0x8000_0000 != 0 {
                    cold_path();
                    return Err(Error::Closed);
                }
                return Ok(Claim::zero());
            } else if Q::FIXED && n > entries {
                cold_path();
                return Err(Error::NotEnoughSpace);
            }

            let n = n.min(entries);

            let new_head = (old_head + n) & (N - 1) as u32;
            if S::IS_MULTI {
                match self
                    .prod_headtail
                    .compare_exchange_weak_head(old_head, new_head, Relaxed, Relaxed)
                {
                    Ok(_) => return Ok(Claim::many(n, old_head)),
                    Err(x) => {
                        cold_path();
                        old_head = x;
                    }
                }
            } else {
                self.prod_headtail.store_head(new_head, Relaxed);
                return Ok(Claim::many(n, old_head));
            }
        }
    }

    fn return_claim_prod(&self, claim: Claim) {
        if S::IS_MULTI {
            self.prod_headtail
                .wait_until_equal_tail(claim.start, Relaxed);
        }

        self.prod_headtail
            .store_tail(claim.new_tail::<N>(), Relaxed);
    }

    /// Move the consumers head to get `n` entries.
    ///
    /// # Returns
    /// The amount of acquired entries, which is smaller or equal to `n` and can be zero.
    fn claim_cons<Q>(&self, n: u32) -> Result<Claim, Error>
    where
        Q: QueueBehaviour,
    {
        let mut old_head = self.cons_headtail.load_head(Relaxed);

        loop {
            // Ensure the head is read before the tail
            fence(Acquire);

            // load-acquire synchronize with store-release of cons_update_tail
            let prod_tail = self.prod_headtail.load_tail(Acquire);

            let entries = Self::initialized_entries(old_head, prod_tail);

            if entries == 0 {
                cold_path();
                // Check if the MSB is set, as that indicates the channel is closed on the other side
                if prod_tail & 0x8000_0000 != 0 {
                    cold_path();
                    return Err(Error::Closed);
                }
                return Ok(Claim::zero());
            } else if Q::FIXED && n > entries {
                cold_path();
                return Err(Error::NotEnoughItems);
            }

            let n = n.min(entries);

            let new_head = (old_head + n) & (N - 1) as u32;
            if R::IS_MULTI {
                match self
                    .cons_headtail
                    .compare_exchange_weak_head(old_head, new_head, Relaxed, Relaxed)
                {
                    Ok(_) => return Ok(Claim::many(n, old_head)),
                    Err(x) => {
                        cold_path();
                        old_head = x;
                    }
                }
            } else {
                self.cons_headtail.store_head(new_head, Relaxed);
                return Ok(Claim::many(n, old_head));
            }
        }
    }

    fn return_claim_cons(&self, claim: Claim) {
        if R::IS_MULTI {
            self.cons_headtail
                .wait_until_equal_tail(claim.start, Relaxed);
        }

        self.cons_headtail
            .store_tail(claim.new_tail::<N>(), Relaxed);
    }

    /// Get a reference to the data part of the ring.
    fn data(&self) -> &[UnsafeCell<MaybeUninit<T>>; N] {
        self.data.deref()
    }

    pub fn try_enqueue<I, Q>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
        Q: QueueBehaviour,
    {
        let claim = match self.claim_prod::<Q>(values.len() as u32) {
            Ok(claim) => claim,
            Err(err) => {
                cold_path();
                return Err(err);
            }
        };

        if claim.entries() == 0 {
            // TODO: Do this in claim
            cold_path();
            return Err(Error::Full);
        }

        let data = self.data();
        for (i, value) in values.take(claim.entries() as usize).enumerate() {
            let mut offset = i + claim.start() as usize;
            if offset >= N {
                cold_path();
                offset -= N;
            }
            // SAFETY: Our Claim gives exclusive access to this index
            unsafe {
                data[offset].get().write(MaybeUninit::new(value));
            }
        }

        let n = claim.entries() as usize;

        self.return_claim_prod(claim);

        Ok(n)
    }

    pub fn try_dequeue<Q>(&self, n: usize) -> Result<RecvValues<N, T, P, C, S, R>, Error>
    where
        Q: QueueBehaviour,
    {
        let claim = match self.claim_cons::<Q>(n as u32) {
            Ok(claim) => claim,
            Err(err) => {
                cold_path();
                return Err(err);
            }
        };

        if claim.entries() == 0 {
            cold_path();
            // TODO: Do this in claim
            return Err(Error::Empty);
        }

        unsafe { Ok(RecvValues::new(self, claim)) }
    }

    const fn initialized_entries(cons_head: u32, prod_tail: u32) -> u32 {
        // Clear the MSB in case the producers are already dropped
        let prod_tail = prod_tail & 0x7FFF_FFFF;
        prod_tail.wrapping_sub(cons_head) & (N as u32 - 1)
    }

    const fn uninitialized_entries(prod_head: u32, cons_tail: u32) -> u32 {
        // Clear the MSB in case the producers are already dropped
        let cons_tail = cons_tail & 0x7FFF_FFFF;
        (N as u32 - 1 + cons_tail).wrapping_sub(prod_head) & (N as u32 - 1)
    }

    pub fn poison(&self) {
        // TODO: also poison headtails
        self.active.poison();
    }
}

/// Functions for keeping track of the active consumers and producers.
impl<const N: usize, T, P, C, S, R> Ring<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
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

pub struct Claim {
    // TODO: Maybe make nonzero?
    entries: u32,
    start: u32,
}

impl Claim {
    /// An empty claim for when there are no entries available.
    const fn zero() -> Self {
        Self {
            entries: 0,
            start: 0,
        }
    }

    /// A claim for `n` entries at `start`
    const fn many(entries: u32, start: u32) -> Self {
        Self { entries, start }
    }

    const fn entries(&self) -> u32 {
        self.entries
    }

    const fn start(&self) -> u32 {
        self.start
    }

    const fn new_tail<const N: usize>(self) -> u32 {
        let new = self.start as u64 + self.entries as u64;
        let _dont_drop_self = ManuallyDrop::new(self);
        if new >= N as u64 {
            cold_path();
            (new - N as u64) as u32
        } else {
            new as u32
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        // The Claim should always be consumed by `new_tail` if it has at least one entry.
        // `new_tail` doesn't cause a drop because it uses ManuallyDrop.
        if self.entries != 0 && !std::thread::panicking() {
            cold_path();
            panic!("Claim was dropped before being returned");
        }
    }
}

/// What to do when there is not enough room for (de)queueing all items.
///
/// When `FIXED` is `true`, then it will give up. Otherwise, it will just (de)queue less.
pub trait QueueBehaviour {
    const FIXED: bool;
}
pub enum FixedQueue {}
impl QueueBehaviour for FixedQueue {
    const FIXED: bool = true;
}
pub enum VariableQueue {}

impl QueueBehaviour for VariableQueue {
    const FIXED: bool = false;
}
