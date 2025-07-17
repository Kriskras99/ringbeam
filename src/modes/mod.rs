//! Sync modes for the producers and consumers.

use crate::{
    Error,
    sealed::Sealed,
    std::{hint::cold_path, sync::atomic::Ordering},
};
use std::{
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

#[expect(private_bounds, reason = "We don't want to expose these functions")]
pub trait Mode: ModeInner {}
impl<T: ModeInner> Mode for T {}

pub(crate) trait ModeInner: Default {
    /// Move the head.
    ///
    /// # Generics
    /// - `N`: The ring size.
    /// - `IS_PROD`: Is the headtail a producer.
    /// - `Q`: What to do when there is not enough room for all items, see [`QueueBehaviour`].
    /// - `Other`: The mode of the other headtail on the ring.
    ///
    /// # Errors
    ///
    fn move_head<const N: usize, const IS_PROD: bool, Q: QueueBehaviour, Other: Mode>(
        &self,
        other: &Other,
        n: NonZeroU32,
    ) -> Result<Claim, Error>;
    fn update_tail<const N: usize>(&self, claim: Claim);
    #[must_use]
    fn load_tail(&self, ordering: Ordering) -> u32;
    fn mark_finished(&self);
    #[must_use]
    fn is_finished(&self) -> bool;
}

/// What to do when there is not enough room for (de)queueing all items.
///
/// When `FIXED` is `true`, then it will give up. Otherwise, it will just (de)queue less.
pub trait QueueBehaviour: Sealed {
    const FIXED: bool;
}
pub enum FixedQueue {}
impl Sealed for FixedQueue {}
impl QueueBehaviour for FixedQueue {
    const FIXED: bool = true;
}
pub enum VariableQueue {}
impl Sealed for VariableQueue {}
impl QueueBehaviour for VariableQueue {
    const FIXED: bool = false;
}

#[must_use]
pub struct Claim {
    entries: NonZeroU32,
    start: u32,
}

impl Debug for Claim {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Claim {{ entries: {}, start: {}}}",
            self.entries, self.start
        ))
    }
}

impl Claim {
    /// A claim for `n` entries at `start`
    pub const fn many(entries: NonZeroU32, start: u32) -> Self {
        Self { entries, start }
    }

    #[must_use]
    pub const fn entries(&self) -> u32 {
        self.entries.get()
    }

    #[must_use]
    pub const fn start(&self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn new_tail<const N: usize>(self) -> u32 {
        let new = self.start as u64 + self.entries.get() as u64;
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
fn calculate_available<const N: usize, const IS_PROD: bool, Q: QueueBehaviour>(
    head: u32,
    tail: u32,
    expected: NonZeroU32,
) -> Result<NonZeroU32, Error> {
    let start = if IS_PROD { N as u32 - 1 } else { 0 };
    // Clear the MSB in case the head or tail are already dropped
    let available = start
        .wrapping_add(tail & 0x7FFF_FFFF)
        .wrapping_sub(head & 0x7FFF_FFFF)
        & (N as u32 - 1);
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
    } else if Q::FIXED && expected.get() > available {
        cold_path();
        if IS_PROD {
            Err(Error::NotEnoughSpace)
        } else {
            Err(Error::NotEnoughItems)
        }
    } else {
        Ok(expected.min(NonZeroU32::new(available).unwrap_or_else(|| unreachable!())))
    }
}
