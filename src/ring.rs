use crate::cache_padded::CachePadded;
use crate::consumer::Receiver;
use crate::producer::Sender;
use crate::{Error, HeadTail};
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit, offset_of};
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release, SeqCst};
use std::sync::atomic::{AtomicU32, fence};

pub enum Multi {}
impl IsMulti for Multi {
    const IS_MULTI: bool = true;
}

pub enum Single {}
impl IsMulti for Single {
    const IS_MULTI: bool = false;
}

/// Can more than instance of the sender/receiver exist?
pub trait IsMulti {
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
    active: AtomicU32,
    prod_headtail: CachePadded<P>,
    cons_headtail: CachePadded<C>,
    data: CachePadded<UnsafeCell<[MaybeUninit<T>; N]>>,
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
    pub fn new<ReceiveHalf>() -> (Sender<N, T, P, C, S, R>, ReceiveHalf)
    where
        ReceiveHalf: Receiver<N, T, P, C, S, R>,
    {
        // Check input
        const {
            assert!(
                !(!N.checked_add(1).unwrap().is_power_of_two() && N <= u32::MAX as usize),
                "Requested capacity was not equal to `2.pow(n)-1` where `n >= 1`"
            );
            assert!(
                size_of::<T>().is_multiple_of(4),
                "T must be a multiple of four"
            );
        }

        // Allocate the ring
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        // Initialize the ring
        // SAFETY: this is the only pointer to the data, no references exist
        unsafe {
            ptr.add(offset_of!(Self, active))
                .cast::<CachePadded<AtomicU32>>()
                .write(CachePadded::new(AtomicU32::new(0x0001_0001)));
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
                ReceiveHalf::new_no_register(ring),
            )
        };
        (sender, receiver)
    }

    pub fn register_producer(&self) -> Result<(), Error> {
        // TODO: This ordering is most likely too strict
        self.active
            .fetch_update(SeqCst, SeqCst, |old| {
                if old & 0xFFFF_0000 == 0 || old & 0x0000_FFFF == 0 {
                    None
                } else if old & 0xFFFF_0000 == 0xFFFF_0000 {
                    panic!("Too many producers, would overflow!");
                } else {
                    // Saturating add for only the producer bytes
                    let p = ((old >> 16) as u16) + 1;
                    Some((old & 0x0000_FFFF) | ((p as u32) << 16))
                }
            })
            .map(|_| ())
            .map_err(|_| Error::Closed)
    }

    pub fn register_consumer(&self) -> Result<(), Error> {
        // TODO: This ordering is most likely too strict
        self.active
            .fetch_update(SeqCst, SeqCst, |old| {
                if old & 0xFFFF_0000 == 0 || old & 0x0000_FFFF == 0 {
                    None
                } else if old & 0x0000_FFFF == 0x0000_FFFF {
                    panic!("Too many consumers, would overflow!");
                } else {
                    // Saturating add for only the consumer bytes
                    let p = ((old & 0xFFFF) as u16).saturating_add(1);
                    Some((old & 0xFFFF_0000) | (p as u32))
                }
            })
            .map(|_| ())
            .map_err(|_| Error::Closed)
    }

    /// Unregister an active producer, returning `true` if it was the last one.
    pub fn unregister_producer(&self) -> bool {
        // TODO: This ordering is most likely too strict
        self.active
            .fetch_update(SeqCst, SeqCst, |old| {
                if (old & 0xFFFF_0000) == 0 {
                    panic!("Active producers is zero but trying to unregister an active producer");
                } else {
                    // Sub for only the producer bytes
                    let p = ((old >> 16) as u16) - 1;
                    Some((old & 0x0000_FFFF) | ((p as u32) << 16))
                }
            })
            .unwrap_or_else(|_| unreachable!())
            == 0
    }

    /// Unregister an active consumer, returning `true` if it was the last one.
    pub fn unregister_consumer(&self) -> bool {
        // TODO: This ordering is most likely too strict
        self.active
            .fetch_update(SeqCst, SeqCst, |old| {
                if (old & 0x0000_FFFF) == 0 {
                    panic!("Active consumers is zero but trying to unregister an active consumer");
                } else {
                    // Sub for only the consumer bytes
                    let p = ((old & 0xFFFF) as u16) - 1;
                    Some((old & 0xFFFF_0000) | (p as u32))
                }
            })
            .unwrap_or_else(|_| unreachable!())
            == 0
    }

    pub fn active_producers(&self) -> u16 {
        // TODO: This ordering is most likely too strict
        (self.active.load(SeqCst) >> 16) as u16
    }

    pub fn active_consumers(&self) -> u16 {
        // TODO: This ordering is most likely too strict
        (self.active.load(SeqCst) & 0xFFFF) as u16
    }

    /// Move the producers head to get `n` entries.
    ///
    /// # Returns
    /// The amount of acquired entries, which is smaller or equal to `n` and can be zero.
    fn claim_prod(&self, n: u32) -> Claim {
        let mut old_head = self.prod_headtail.load_head(Release);

        loop {
            // Ensure the head is read before the tail
            fence(Acquire);

            // load-acquire synchronize with store-release of cons_update_tail
            let cons_tail = self.cons_headtail.load_tail(Acquire);

            let entries = N as u32 + cons_tail - old_head;

            if entries == 0 {
                return Claim::zero();
            }

            let n = n.min(entries);

            let new_head = old_head + n;
            if S::IS_MULTI {
                match self
                    .prod_headtail
                    .compare_exchange_weak_head(old_head, new_head, Relaxed, Relaxed)
                {
                    Ok(_) => return Claim::many(n, old_head),
                    Err(x) => old_head = x,
                }
            } else {
                self.prod_headtail.store_head(new_head, Relaxed);
            }
        }
    }

    fn return_claim_prod(&self, claim: Claim) {
        if S::IS_MULTI {
            self.prod_headtail
                .wait_until_equal_tail(claim.start, Relaxed);
        }

        self.prod_headtail.store_tail(claim.new_tail(N), Relaxed);
    }

    /// Get a mutable pointer to the data part of the ring.
    ///
    /// # Safety
    /// The caller *must* have a [`Claim`] and only access that part of the data.
    unsafe fn data(&self) -> *mut MaybeUninit<T> {
        self.data.get().cast()
    }

    pub fn try_enqueue<I, Q>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
        Q: QueueBehaviour,
    {
        let claim = self.claim_prod(values.len() as u32);

        if claim.entries() == 0 {
            if self.active_consumers() == 0 {
                return Err(Error::Closed);
            } else {
                return Err(Error::Full);
            }
        } else if Q::FIXED && claim.entries() != values.len() as u32 {
            return Err(Error::NotEnoughSpace);
        }

        // SAFETY: We have a Claim to a part of the data and only access that part
        let data = unsafe { self.data() };
        for (i, value) in values.take(claim.entries() as usize).enumerate() {
            let mut offset = i + claim.start() as usize;
            if offset >= N {
                offset = 0;
            }
            unsafe {
                data.add(offset).write(MaybeUninit::new(value));
            }
        }

        let n = claim.entries() as usize;

        self.return_claim_prod(claim);

        Ok(n)
    }
}

pub struct Claim {
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

    const fn new_tail(self, capacity: usize) -> u32 {
        let new = self.start + self.entries;
        let _dont_drop_self = ManuallyDrop::new(self);
        let capacity = capacity as u32;
        if new > capacity {
            // TODO: This goes wrong in the overflow
            new - capacity
        } else {
            new
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        // The Claim should always be consumed by `new_tail`, which uses ManuallyDrop.
        // The assert does not trigger if the thread is already panicking.
        assert!(
            std::thread::panicking(),
            "Claim was dropped before being returned"
        );
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

pub struct RecvValues<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    claim_and_ring: Option<(Claim, *const Ring<N, T, P, C, S, R>)>,
    index: u32,
    offset: u32,
}

impl<const N: usize, T, P, C, S, R> RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    fn new(ring: *const Ring<N, T, P, C, S, R>, claim: Claim) -> Self {
        let offset = claim.start();
        Self {
            claim_and_ring: Some((claim, ring)),
            index: 0,
            offset,
        }
    }
}

impl<const N: usize, T, P, C, S, R> Iterator for RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((_claim, ring)) = &self.claim_and_ring {
            // SAFETY: ring is initialized and aligned otherwise this type can't be created
            let ring = unsafe { &**ring };
            // SAFETY: We have a Claim to a part of the data and only access that part
            let data = unsafe { ring.data() };

            todo!()
        } else {
            None
        }
    }
}
