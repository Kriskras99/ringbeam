#![expect(unused, reason = "Still developing")]
mod cache_padded;
mod headtail;

use Ordering::{Acquire, Relaxed, Release};
pub use cache_padded::CachePadded;
use std::marker::PhantomData;
use std::mem::{MaybeUninit, offset_of};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use thiserror::Error;

use headtail::HeadTailInner;
pub use headtail::{HeadTail, HeadTailSync, RelaxedTailSync, TailSync};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Channel is closed")]
    ChannelClosed,
    #[error("Channel is full")]
    ChannelFull,
}

#[expect(private_bounds, reason = "It's a sealed trait")]
pub struct MultiProducer<const N: usize, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, Self, R>,
}

impl<const N: usize, T, P, C, R> SenderInner<N, T, P, C, R> for MultiProducer<N, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    unsafe fn new(ring: *const Ring<N, T, P, C, Self, R>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_producer().unwrap();
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, Self, R>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_producers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    fn try_enqueue_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        let claim = unsafe { (*self.ring).claim_prod(values.len() as u32) };

        if claim.entries == 0 {
            if unsafe { (*self.ring).active_consumers() == 0 } {
                return Err(Error::ChannelClosed);
            } else {
                return Err(Error::ChannelFull);
            }
        }

        let data_start = unsafe { (*self.ring).data.as_ptr() }.cast_mut();
        for (i, value) in values.take(claim.entries as usize).enumerate() {
            let mut offset = i + claim.start as usize;
            if offset >= N {
                offset = 0;
            }
            unsafe {
                data_start.add(offset).write(MaybeUninit::new(value));
            }
        }

        let n = claim.entries as usize;

        unsafe { (*self.ring).return_claim_prod(claim) };

        Ok(n)
    }
}
impl<const N: usize, T, P, C, R> Clone for MultiProducer<N, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}
impl<const N: usize, T, P, C, R> Drop for MultiProducer<N, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    fn drop(&mut self) {
        unsafe {
            if (*self.ring).unregister_producer() {
                // TODO: unallocate the ring
            }
        }
    }
}

#[expect(private_bounds, reason = "It's a sealed trait")]
pub struct SingleProducer<const N: usize, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, Self, R>,
}
impl<const N: usize, T, P, C, R> SenderInner<N, T, P, C, R> for SingleProducer<N, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    const IS_SP: bool = true;

    unsafe fn new(ring: *const Ring<N, T, P, C, Self, R>) -> Self {
        // As only 1 SingleProducer is allowed to exist this would require ring.active_producers
        // to be zero, but that would mean the channel is closed.
        panic!("SingleProducer cannot be created through SenderInner::new");
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, Self, R>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_producers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    fn try_enqueue_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator,
    {
        todo!()
    }
}

#[expect(private_bounds, reason = "It's a sealed trait")]
pub struct MultiConsumer<const N: usize, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, S, Self>,
}
impl<const N: usize, T, P, C, S> ReceiverInner<N, T, P, C, S> for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
{
    unsafe fn new(ring: *const Ring<N, T, P, C, S, Self>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            (*ring).register_consumer().unwrap();
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, Self>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }
}
impl<const N: usize, T, P, C, S> Clone for MultiConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
{
    fn clone(&self) -> Self {
        // SAFETY: because `self` is valid, `ring` is initialized and aligned.
        unsafe { Self::new(self.ring) }
    }
}
#[expect(private_bounds, reason = "It's a sealed trait")]
pub struct SingleConsumer<const N: usize, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
{
    phantom: PhantomData<T>,
    ring: *const Ring<N, T, P, C, S, Self>,
}
impl<const N: usize, T, P, C, S> ReceiverInner<N, T, P, C, S> for SingleConsumer<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
{
    unsafe fn new(ring: *const Ring<N, T, P, C, S, Self>) -> Self {
        // As only 1 SingleConsumer is allowed to exist this would require ring.active_consumers
        // to be zero, but that would mean the channel is closed.
        panic!("SingleConsumer cannot be created through ReceiverInner::new");
    }

    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, Self>) -> Self {
        // SAFETY: caller has assured that `ring` is initialized and aligned.
        unsafe {
            debug_assert!((*ring).active_consumers() == 1);
        }
        Self {
            ring,
            phantom: PhantomData,
        }
    }
}

trait SenderInner<const N: usize, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
    Self: Sized,
{
    /// Is the sender the only producer.
    const IS_SP: bool = false;

    /// Create a new sender.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    unsafe fn new(ring: *const Ring<N, T, P, C, Self, R>) -> Self;

    /// Create a new sender but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active senders counter must have already been incremented.
    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, Self, R>) -> Self;

    #[inline(always)]
    fn try_enqueue(&self, value: T) -> Result<usize, Error> {
        self.try_enqueue_bulk(&mut std::iter::once(value))
    }

    /// Try to put all values into the ring.
    ///
    /// The implementation will only consume as many values as it can fit into the ring.
    ///
    /// # Returns
    /// The amount of values written
    // TODO: The Iterator must be TrustedLen, but that's unstable
    fn try_enqueue_bulk<I>(&self, values: &mut I) -> Result<usize, Error>
    where
        I: Iterator<Item = T> + ExactSizeIterator;
}

#[expect(private_bounds, reason = "It's a sealed trait")]
pub trait Sender<const N: usize, T, P, C, R>: SenderInner<N, T, P, C, R>
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
{
    fn try_send(&self, msg: T) -> Result<usize, Error>;
}
impl<const N: usize, T, P, C, R, Inner> Sender<N, T, P, C, R> for Inner
where
    P: HeadTail,
    C: HeadTail,
    R: ReceiverInner<N, T, P, C, Self>,
    Inner: SenderInner<N, T, P, C, R>,
{
    fn try_send(&self, msg: T) -> Result<usize, Error> {
        self.try_enqueue(msg)
    }
}

trait ReceiverInner<const N: usize, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: SenderInner<N, T, P, C, Self>,
    Self: Sized,
{
    /// Create a new receiver.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`].
    unsafe fn new(ring: *const Ring<N, T, P, C, S, Self>) -> Self;

    /// Create a new receiver but don't register it as active.
    ///
    /// This should only be used when initializing the ring.
    ///
    /// # Safety
    /// `ring` must point to an initialized and aligned [`Ring`]. In addition,
    /// the active receivers counter must have already been incremented.
    unsafe fn new_no_register(ring: *const Ring<N, T, P, C, S, Self>) -> Self;
}

#[expect(private_bounds, reason = "It's a sealed trait")]
pub trait Receiver<const N: usize, T, P, C, S>: ReceiverInner<N, T, P, C, S>
where
    P: HeadTail,
    C: HeadTail,
    S: Sender<N, T, P, C, Self>,
{
}
impl<const N: usize, T, P, C, S, Inner> Receiver<N, T, P, C, S> for Inner
where
    P: HeadTail,
    C: HeadTail,
    S: Sender<N, T, P, C, Self>,
    Inner: ReceiverInner<N, T, P, C, S>,
{
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
struct Ring<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: Sender<N, T, P, C, R>,
    R: Receiver<N, T, P, C, S>,
{
    phantom: PhantomData<(T, S, R)>,
    /// Active producers and consumers.
    ///
    /// Where the first u16 is the producers and the second u16 is the consumers (`0xPPPP_CCCC`).
    active: AtomicU32,
    prod_headtail: CachePadded<P>,
    cons_headtail: CachePadded<C>,
    data: CachePadded<[MaybeUninit<T>; N]>,
}

impl<const N: usize, T, P, C, S, R> Ring<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: Sender<N, T, P, C, R>,
    R: Receiver<N, T, P, C, S>,
{
    /// Create the ring returning a sender and receiver.
    #[expect(
        clippy::new_ret_no_self,
        reason = "This type should only be used through the sender and receiver"
    )]
    pub fn new() -> (S, R) {
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
        let (sender, receiver) = unsafe { (S::new_no_register(ring), R::new_no_register(ring)) };
        (sender, receiver)
    }

    fn register_producer(&self) -> Result<(), Error> {
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
            .map_err(|_| Error::ChannelClosed)
    }

    fn register_consumer(&self) -> Result<(), Error> {
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
            .map_err(|_| Error::ChannelClosed)
    }

    /// Unregister an active producer, returning `true` if it was the last one.
    fn unregister_producer(&self) -> bool {
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
    fn unregister_consumer(&self) -> bool {
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

    fn active_producers(&self) -> u16 {
        // TODO: This ordering is most likely too strict
        (self.active.load(SeqCst) >> 16) as u16
    }

    fn active_consumers(&self) -> u16 {
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

            let new_head = old_head + entries;
            if S::IS_SP {
                self.prod_headtail.store_head(new_head, Relaxed);
            } else {
                match self
                    .prod_headtail
                    .compare_exchange_weak_head(old_head, new_head, Relaxed, Relaxed)
                {
                    Ok(_) => return Claim::many(entries, old_head),
                    Err(x) => old_head = x,
                }
            }
        }
    }

    fn return_claim_prod(&self, claim: Claim) {
        if S::IS_SP {
            self.prod_headtail
                .wait_until_equal_tail(claim.start, Relaxed);
        }

        self.prod_headtail.store_tail(claim.new_tail(N), Relaxed);
    }
}

struct Claim {
    entries: u32,
    start: u32,
}

impl Claim {
    /// An empty claim for when there are no entries available.
    #[inline(always)]
    const fn zero() -> Self {
        Self {
            entries: 0,
            start: 0,
        }
    }

    /// A claim for `n` entries at `start`
    #[inline(always)]
    const fn many(entries: u32, start: u32) -> Self {
        Self { entries, start }
    }

    const fn new_tail(&self, capacity: usize) -> u32 {
        let new = self.start + self.entries;
        let capacity = capacity as u32;
        if new > capacity {
            // TODO: This goes wrong in the overflow
            new - capacity
        } else {
            new
        }
    }
}
