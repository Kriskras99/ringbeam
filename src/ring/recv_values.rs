//! Logic for reading from a channel through a iterator.
use crate::{
    Error,
    modes::{Claim, Mode},
    ring::{Ring, active::Last},
    std::hint::cold_path,
};

/// A view into a part of the channel.
///
/// The items can be consumed by using its iterator implementation.
/// If this is dropped before being fully consumed, the items it can view
/// will also be dropped.
pub struct RecvValues<const N: usize, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// What data we're allowed to access and the ring to access it in.
    ///
    /// If this is `None`, we either never had a claim or we've finished the claim.
    claim_and_ring: Option<(Claim, *const Ring<N, T, P, C>)>,
    /// The amount of items already consumed
    consumed: u32,
    /// Offset (in amount of `T`) in `Ring::data()` where the next item is.
    ///
    /// This must always be valid while `claim_and_ring` is `Some`.
    offset: u32,
}

impl<const N: usize, T, P, C> RecvValues<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    /// Create a new value iterator.
    ///
    /// # Safety
    /// `ring` must point to a valid, aligned [`Ring`]. This should always be the case if the caller
    /// is currently registered with the ring.
    ///
    /// # Errors
    /// Will return [`Error::Poisoned`] if the ring is poisoned and [`Error::TooManyConsumers`] if
    /// there are already `u16::MAX - 1` consumers.
    #[inline]
    pub(crate) unsafe fn new(ring: *const Ring<N, T, P, C>, claim: Claim) -> Result<Self, Error> {
        // SAFETY: Caller guarantees the ring is valid
        unsafe {
            (&*ring).active().register_consumer()?;
        }
        let offset = claim.start();
        Ok(Self {
            claim_and_ring: Some((claim, ring)),
            consumed: 0,
            offset,
        })
    }

    /// Create a new empty [`RecvValues`].
    ///
    /// It won't be registered in any ring and `Self::next` will always return `None`.
    #[inline]
    pub(crate) const fn new_empty() -> Self {
        Self {
            claim_and_ring: None,
            consumed: 0,
            offset: 0,
        }
    }
}

impl<const N: usize, T, P, C> Iterator for RecvValues<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    type Item = T;

    #[expect(
        clippy::missing_inline_in_public_items,
        reason = "This function is too large too inline"
    )]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some((claim, ring)) = self.claim_and_ring.take() {
            // SAFETY: RecvValues is registered as a consumer, so ring is a valid reference
            //         The Claim guarantees we have exclusive access to this index and that
            //         there is a valid, initialized item at the index.
            let value = unsafe {
                (*ring).data()[self.offset as usize].with_mut(|p| (*p).assume_init_take())
            };

            self.consumed += 1;
            self.offset = self.offset.wrapping_add(1) & (N as u32 - 1);
            if self.consumed >= claim.entries() {
                cold_path();
                // SAFETY: We're still registered so the ring must be valid
                unsafe {
                    (*ring).return_claim_cons(claim);
                }
                // SAFETY: We're still registered so the ring must be valid
                match unsafe {
                    (*ring)
                        .active()
                        .unregister_consumer()
                        .expect("Ring is poisoned!")
                } {
                    Last::InCategory => {
                        // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                        //         wait for the tail being marked.
                        unsafe {
                            (*ring).mark_cons_finished();
                        }
                    }
                    Last::InRing => {
                        // SAFETY: `Last::InRing` guarantees that we're the last
                        unsafe {
                            Ring::cleanup(ring);
                        }
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

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = if let Some((claim, _)) = &self.claim_and_ring {
            (claim.entries() - self.consumed) as usize
        } else {
            cold_path();
            0
        };
        (left, Some(left))
    }
}

impl<const N: usize, T, P, C> Drop for RecvValues<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
    #[expect(
        clippy::missing_inline_in_public_items,
        reason = "This function is too large too inline"
    )]
    fn drop(&mut self) {
        if let Some((claim, ring)) = self.claim_and_ring.take() {
            cold_path();
            while self.consumed != claim.entries() {
                // SAFETY: Ring is valid while we haven't unregistered.
                //         The Claim guarantees we have exclusive access to this index and that
                //         there is a valid, intialized item at the index.
                unsafe {
                    (*ring).data()[self.offset as usize].with_mut(|p| (*p).assume_init_drop());
                };
                self.consumed += 1;
                self.offset = self.offset.wrapping_add(1) & (N as u32 - 1);
            }

            // SAFETY: We're still registered so the ring must be valid
            unsafe {
                (*ring).return_claim_cons(claim);
            }
            // SAFETY: We're still registered so the ring must be valid
            match unsafe {
                (*ring)
                    .active()
                    .unregister_consumer()
                    .expect("Ring was poisoned!")
            } {
                Last::InCategory => {
                    // SAFETY: Even if another thread starts the ring cleanup, the cleanup will
                    //         wait for the tail being marked.
                    unsafe {
                        (*ring).mark_cons_finished();
                    }
                }
                Last::InRing => {
                    // SAFETY: `Last::InRing` guarantees that we're the last
                    unsafe {
                        Ring::cleanup(ring);
                    }
                }
                Last::NotLast => {}
            }
        }
    }
}

impl<const N: usize, T, P, C> ExactSizeIterator for RecvValues<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
}

#[cfg(feature = "trusted_len")]
// SAFETY: The ExactSizeIterator implementation is always accurate
unsafe impl<const N: usize, T, P, C> core::iter::TrustedLen for RecvValues<N, T, P, C>
where
    P: Mode,
    C: Mode,
{
}
