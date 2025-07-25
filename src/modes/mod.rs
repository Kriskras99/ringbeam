//! Sync modes for the producers and consumers.

use crate::{
    Error,
    std::{hint::cold_path, sync::atomic::Ordering},
};
use core::{
    fmt::{Debug, Formatter},
    mem::ManuallyDrop,
    num::NonZeroU32,
};

mod hts;
mod multi;
mod rts;
mod single;

pub use hts::HeadTailSync;
pub use multi::Multi;
pub use rts::RelaxedTailSync;
pub use single::Single;

/// The synchronisation mode of a [`Sender`](crate::custom::Sender) or [`Receiver`](crate::custom::Receiver).
///
/// The channel contains a large ring of slots where the values are put by the `Receiver` and read
/// by the `Sender`. Both the `Sender` and `Receiver` halfs of the channel keep track of their progress
/// using two indexes: the head and the tail. The `Sender` is currently writing values between its
/// tail and head, and the `Receiver` is currently reading the values between its tail and head.
/// Between the `Sender` tail and the `Receiver` head are values that have not been written yet.
/// Between the `Receiver` tail and the `Sender` head are empty slots.
///
/// As the `Sender` and `Receiver` can be on different threads and depending on the mode there can
/// be multiple of each on different threads. Therefore, the 'headtails' must be accessed in a
/// synchronised way otherwise data races and undefined behaviour start occurring.
///
/// The different modes allows the user to choose the synchronisation method that is best for their
/// specific situation.
///
/// There are currently four modes:
/// - [`Single`]: Only allows singlethreaded access to a 'headtail'.
/// - [`Multi`]: Allows multithreaded access to a 'headtail'. Every thread spins on the head to acquire
///   slots. After they're done they spin on the tail to update past their slots.
/// - [`HeadTailSync`]: Allows multithreaded access but only one thread is allowed to update the head.
///   Only after it's done with the slots and updated the tail the next thread can update the head.
/// - [`RelaxedTailSync`]: Similar to `Multi`, but only the last thread updates the tail.
pub trait Mode: ModeInner {
    /// The settings for this mode.
    ///
    /// Currently only relevant for [`RelaxedTailSync`].
    type Settings: Default;

    /// Create the mode with custom settings.
    fn new_with(settings: Self::Settings) -> Self;
}

/// Represents the head and tail.
///
/// Can be implemented in various ways, see [`Mode`].
pub trait ModeInner: Default {
    /// Move the head.
    ///
    /// # Generics
    /// - `N`: The ring size.
    /// - `IS_PROD`: Is the headtail a producer.
    /// - `EXACT`: Does the caller want exactly `expected` items, or is fewer also fine.
    /// - `Other`: The mode of the other headtail on the ring.
    ///
    /// # Errors
    /// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
    /// one of those states. The last one indicates that retrying can be successful. If `EXACT` it
    /// can also return [`Error::NotEnoughSpace`]/[`Error::NotEnoughItems`],
    /// which can also be successful on a retry. If `IS_PROD` it can also return [`Error::NotEnoughItemsAndClosed`]
    /// which can be successful on a retry with `EXACT: false`.
    fn move_head<const N: usize, const IS_PROD: bool, const EXACT: bool, Other: Mode>(
        &self,
        other: &Other,
        expected: NonZeroU32,
    ) -> Result<Claim, Error>;

    /// Return the claim and move the tail forward.
    fn update_tail<const N: usize>(&self, claim: Claim);

    /// Load the tail value with the specified ordering.
    #[must_use]
    fn load_tail(&self, ordering: Ordering) -> u32;

    /// Mark this head as finished.
    ///
    /// This should only be called by the last owner as indicated by [`Last::InCategory`](crate::ring::active::Last),
    /// or when the ring is poisoned.
    fn mark_finished(&self);

    /// Have all owners of the head finished.
    ///
    /// If this is `true` then the head won't move anymore.
    #[must_use]
    fn is_finished(&self) -> bool;
}

/// A unique claim to a part of the ring.
///
/// Can be acquired using [`ModeInner::move_head`]. When acquired with `IS_PROD: true` then the
/// entries are uninitialized and can only be written too. If `IS_PROD: false` then the entries
/// can only be read once each.
///
/// A claim **must** be fully consumed before being returned.
#[must_use]
pub struct Claim {
    /// The amount of entries from `start` which are part of the claim.
    entries: NonZeroU32,
    /// The place in the ring where the claim starts.
    start: u32,
}

impl Debug for Claim {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "Claim {{ entries: {}, start: {}}}",
            self.entries, self.start
        ))
    }
}

impl Claim {
    /// A claim for `n` entries at `start`
    #[inline]
    pub const fn many(entries: NonZeroU32, start: u32) -> Self {
        Self { entries, start }
    }

    /// The amount of entries in the claim.
    ///
    /// Guaranteed to be non-zero.
    #[must_use]
    #[inline]
    pub const fn entries(&self) -> u32 {
        self.entries.get()
    }

    /// The start of the claim.
    #[must_use]
    #[inline]
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// Calculate the new location of the tail.
    #[must_use]
    #[inline]
    pub const fn new_tail<const N: usize>(self) -> u32 {
        let new = self.start.wrapping_add(self.entries.get()) & (N as u32 - 1);
        let _dont_drop_self = ManuallyDrop::new(self);
        new
    }
}

impl Drop for Claim {
    #[inline]
    fn drop(&mut self) {
        // The Claim should always be consumed by `new_tail` if it has at least one entry.
        // `new_tail` doesn't cause a drop because it uses ManuallyDrop.
        assert!(
            std::thread::panicking(),
            "Claim was dropped before being returned"
        );
    }
}

/// Calculate the available entries (either occupied or empty).
///
/// # Generics
/// - `N`: The ring size.
/// - `M`: The ring size when calculating for a producer, 0 when calculating for a consumer.
///
/// # Errors
/// Can return [`Error::Closed`], [`Error::Poisoned`], or [`Error::Empty`] if the ring is in
/// one of those states. The last one indicates that retrying can be successful. If `EXACT` it can
/// also return [`Error::NotEnoughSpace`]/[`Error::NotEnoughItems`], which can also be successful on
/// a retry. If `IS_PROD` it can also return [`Error::NotEnoughItemsAndClosed`] which can be successful
/// on a retry with `EXACT: false`.
fn calculate_available<const N: usize, const IS_PROD: bool, const EXACT: bool>(
    head: u32,
    tail: u32,
    expected: NonZeroU32,
) -> Result<NonZeroU32, Error> {
    let start = if IS_PROD { N as u32 - 1 } else { 0 };
    // When this is a producer head, check that there still are consumers
    if IS_PROD && tail & 0x8000_0000 != 0 {
        return Err(Error::Closed);
    }
    if head & 0x8000_0000 != 0 {
        return Err(Error::Poisoned);
    }
    // Clear the MSB in case the tail is already dropped
    let available = start.wrapping_add(tail & 0x7FFF_FFFF).wrapping_sub(head) & (N as u32 - 1);
    if available == 0 {
        cold_path();
        // Check if the MSB is set, as that indicates the channel is closed on the other side
        if tail & 0x8000_0000 != 0 {
            cold_path();
            Err(Error::Closed)
        } else if IS_PROD {
            Err(Error::Full)
        } else {
            Err(Error::Empty)
        }
    } else if EXACT && expected.get() > available {
        cold_path();
        if IS_PROD {
            Err(Error::NotEnoughSpace)
        } else if tail & 0x8000_0000 != 0 {
            cold_path();
            Err(Error::NotEnoughItemsAndClosed)
        } else {
            Err(Error::NotEnoughItems)
        }
    } else {
        Ok(expected.min(NonZeroU32::new(available).unwrap_or_else(|| unreachable!())))
    }
}
