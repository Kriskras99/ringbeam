use crate::{
    HeadTail,
    atomic::{Ordering::SeqCst, fence},
    cold_path,
    ring::{Claim, IsMulti, Ring, active::Last},
};

/// A view into a part of the channel.
///
/// The items can be consumed by using its iterator implementation.
/// If this is dropped before being fully consumed, the items it can view
/// will also be dropped.
pub struct RecvValues<const N: usize, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    claim_and_ring: Option<(Claim, *const Ring<N, T, P, C, S, R>)>,
    /// The amount of items already consumed
    consumed: u32,
    /// Offset (in amount of `T`) in `Ring::data()` where the next item is.
    ///
    /// This must always be valid while `claim_and_ring` is `Some`.
    offset: u32,
}

impl<const N: usize, T, P, C, S, R> RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    /// Create a new value iterator.
    ///
    /// # Safety
    /// `Claim` *must* contain non-zero entries.
    pub unsafe fn new(ring: *const Ring<N, T, P, C, S, R>, claim: Claim) -> Self {
        fence(SeqCst);
        unsafe {
            // TODO: This is reachable if the channel is poisoned
            (&*ring)
                .register_consumer()
                .unwrap_or_else(|_| unreachable!());
        }
        let offset = claim.start();
        Self {
            claim_and_ring: Some((claim, ring)),
            consumed: 0,
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
        if let Some((claim, ring)) = self.claim_and_ring.take() {
            // SAFETY: RecvValues is registered as a consumer, so ring is a valid reference
            //         The Claim guarantees we have exclusive access to this index and that
            //         there is a valid, intialized item at the index.
            let value = unsafe {
                (*ring).data()[self.offset as usize]
                    .get()
                    .cast_const()
                    .read()
                    .assume_init()
            };

            self.consumed += 1;
            self.offset += 1;
            if self.offset as usize >= N {
                cold_path();
                self.offset = 0;
            }
            if self.consumed >= claim.entries() {
                cold_path();
                // SAFETY: We haven't deregistered yet
                unsafe { (*ring).return_claim_cons(claim) };
                match unsafe { (*ring).unregister_consumer().unwrap() } {
                    Last::InCategory => {
                        // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                        //         wait for the tail being marked.
                        unsafe { (*ring).mark_cons_finished() };
                    }
                    Last::InRing => {
                        // Drop the ring as we're the last
                        unsafe { Ring::cleanup(ring) };
                    }
                    Last::NotLast => {}
                }
            } else {
                self.claim_and_ring = Some((claim, ring));
            }
            Some(value)
        } else {
            cold_path();
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = if let Some((claim, _)) = &self.claim_and_ring {
            (claim.entries() - self.offset) as usize
        } else {
            cold_path();
            0
        };
        (left, Some(left))
    }
}

impl<const N: usize, T, P, C, S, R> Drop for RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    fn drop(&mut self) {
        if let Some((claim, ring)) = self.claim_and_ring.take() {
            while self.consumed != claim.entries() {
                // SAFETY: Ring is valid while we haven't unregistered
                let ring = unsafe { &*ring };
                // SAFETY: The Claim guarantees we have exclusive access to this index and that
                //         there is a valid, intialized item at the index.
                unsafe {
                    ring.data()[self.offset as usize]
                        .get()
                        .cast_const()
                        .read()
                        .assume_init_drop();
                };
                self.consumed += 1;
                self.offset += 1;
                if self.offset as usize >= N {
                    cold_path();
                    self.offset = 0;
                }
            }

            // SAFETY: We haven't deregistered yet
            unsafe { (*ring).return_claim_cons(claim) };
            match unsafe { (*ring).unregister_consumer().unwrap() } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    //         wait for the tail being marked.
                    unsafe { (*ring).mark_cons_finished() };
                }
                Last::InRing => {
                    // Drop the ring as we're the last
                    unsafe { Ring::cleanup(ring) };
                }
                Last::NotLast => {}
            }
        }
    }
}

impl<const N: usize, T, P, C, S, R> ExactSizeIterator for RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
}

#[cfg(feature = "trusted_len")]
// SAFETY: The ExactSizeIterator implementation is always accurate
unsafe impl<const N: usize, T, P, C, S, R> std::iter::TrustedLen for RecvValues<N, T, P, C, S, R>
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
}
