#![doc = include_str!("../README.md")]
#![cfg_attr(feature = "trusted_len", feature(trusted_len))]
#![cfg_attr(feature = "likely", feature(cold_path))]

#[cfg(all(feature = "loom", feature = "shuttle"))]
compile_error!("Features 'loom' and 'shuttle' cannot be enabled at the same time");

mod cache_padded;
mod consumer;
mod modes;
mod producer;
mod ring;
mod std;

// TODO: Use consistent naming for producer/consumer or sender/receiver throughout.
// TODO: Use consistent naming for enqueue/dequeue or send/recv throughout.
// TODO: Implement peek for single/multi_hts
// TODO: Make testing with loom and shuttle actually work
// TODO: Maybe repr(c) on Ring, take an extra look at cache alignment.
// TODO: WFE/SEV on ARM
// TODO: Document the inner workings of the various modes in their module documentation.

/// All errors that can be returned when accessing the channel.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The channel is closed.
    Closed,
    /// The channel is empty.
    Empty,
    /// The channel is full.
    Full,
    /// The caller requested exactly `n` items, but there were not enough items in the channel.
    NotEnoughItems,
    /// The caller requested exactly `n` items, but the channel is closed and only has fewer items left.
    NotEnoughItemsAndClosed,
    /// The caller wants to put exactly `n` items in the channel, but there is not enough room.
    NotEnoughSpace,
    /// A panic occurred while holding access to the channel, so the channel is in an undefined state.
    Poisoned,
    /// There are too many consumers, a new one can't be added.
    ///
    /// The current limit is `u16::MAX - 1`
    TooManyConsumers,
    /// There are too many producers, a new one can't be added.
    ///
    /// The current limit is `u16::MAX - 1`
    TooManyProducers,
}

impl core::error::Error for Error {}

impl core::fmt::Display for Error {
    #[expect(
        clippy::missing_inline_in_public_items,
        reason = "Error formatting is not performance sensitive"
    )]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Closed => f.write_str("Channel is closed"),
            Self::Empty => f.write_str("Channel is empty"),
            Self::Full => f.write_str("Channel is full"),
            Self::NotEnoughItems => f.write_str("Channel had items, but not as many as requested"),
            Self::NotEnoughItemsAndClosed => {
                f.write_str("Channel is closed but still had items, but not as many as requested")
            }
            Self::NotEnoughSpace => {
                f.write_str("Channel had room, but not enough room for all the items")
            }
            Self::Poisoned => f.write_str("Channel is poisoned"),
            Self::TooManyConsumers => {
                f.write_str("Maximum amount of consumers in channel has been reached")
            }
            Self::TooManyProducers => {
                f.write_str("Maximum amount of producers in channel has been reached")
            }
        }
    }
}

/// A channel with a custom configuration.
pub mod custom {
    pub use crate::{consumer::Receiver, producer::Sender, ring::recv_values::RecvValues};
    use crate::{modes::Mode, ring::Ring};

    /// The synchronisation modes that can be used with the custom channel.
    pub mod modes {
        pub use crate::modes::{HeadTailSync, Mode, Multi, RelaxedTailSync, Single};
    }

    /// Create a custom channel with space for `N` values of `T`.
    ///
    /// # Type parameters
    /// - N: the size of the channel,
    /// - T: the type that will be sent over the channel,
    /// - P: the sync mode of the producer head and tail (see [`Mode`]),
    /// - C: the sync mode of the consumer head and tail (see [`Mode`]),
    #[must_use]
    #[inline]
    pub fn bounded<const N: usize, T, P, C>() -> (Sender<N, T, P, C>, Receiver<N, T, P, C>)
    where
        P: Mode,
        C: Mode,
    {
        Ring::new()
    }
}

/// A single-producer single-consumer channel.
pub mod spsc {
    use crate::{modes::Single, ring::Ring};

    /// The receiving half of a bounded single-producer single-consumer channel.
    pub type Receiver<const N: usize, T> = crate::consumer::Receiver<N, T, Single, Single>;

    /// The sending half of a bounded single-producer single-consumer channel.
    pub type Sender<const N: usize, T> = crate::producer::Sender<N, T, Single, Single>;

    /// An iterator over the values read by a [`Receiver`].
    pub type RecvValues<const N: usize, T> =
        crate::ring::recv_values::RecvValues<N, T, Single, Single>;

    /// Create a single-producer single-consumer channel with space for `N` values of `T`.
    #[must_use]
    #[inline]
    pub fn bounded<const N: usize, T>() -> (Sender<N, T>, Receiver<N, T>) {
        Ring::new()
    }
}

/// A single-producer multi-consumer channel.
pub mod spmc {
    use crate::{
        modes::{Multi, Single},
        ring::Ring,
    };

    /// The receiving half of a bounded single-producer multi-consumer channel.
    pub type Receiver<const N: usize, T> = crate::consumer::Receiver<N, T, Single, Multi>;

    /// The sending half of a bounded single-producer multi-consumer channel.
    pub type Sender<const N: usize, T> = crate::producer::Sender<N, T, Single, Multi>;

    /// An iterator over the values read by a [`Receiver`].
    pub type RecvValues<const N: usize, T> =
        crate::ring::recv_values::RecvValues<N, T, Single, Multi>;

    /// Create a single-producer multi-consumer channel with space for `N` values of `T`.
    #[must_use]
    #[inline]
    pub fn bounded<const N: usize, T>() -> (Sender<N, T>, Receiver<N, T>) {
        Ring::new()
    }
}

/// A multi-producer single-consumer channel.
pub mod mpsc {
    use crate::{
        modes::{Multi, Single},
        ring::Ring,
    };

    /// The receiving half of a bounded multi-producer single-consumer channel.
    pub type Receiver<const N: usize, T> = crate::consumer::Receiver<N, T, Multi, Single>;

    /// The sending half of a bounded multi-producer single-consumer channel.
    pub type Sender<const N: usize, T> = crate::producer::Sender<N, T, Multi, Single>;

    /// An iterator over the values read by a [`Receiver`].
    pub type RecvValues<const N: usize, T> =
        crate::ring::recv_values::RecvValues<N, T, Multi, Single>;

    /// Create a multi-producer single-consumer channel with space for `N` values of `T`.
    #[must_use]
    #[inline]
    pub fn bounded<const N: usize, T>() -> (Sender<N, T>, Receiver<N, T>) {
        Ring::new()
    }
}

/// A multi-producer multi-consumer channel.
pub mod mpmc {
    use crate::{modes::Multi, ring::Ring};

    /// The receiving half of a bounded multi-producer multi-consumer channel.
    pub type Receiver<const N: usize, T> = crate::consumer::Receiver<N, T, Multi, Multi>;

    /// The sending half of a bounded multi-producer multi-consumer channel.
    pub type Sender<const N: usize, T> = crate::producer::Sender<N, T, Multi, Multi>;

    /// An iterator over the values read by a [`Receiver`].
    pub type RecvValues<const N: usize, T> =
        crate::ring::recv_values::RecvValues<N, T, Multi, Multi>;

    /// Create a multi-producer multi-consumer channel with space for `N` values of `T`.
    #[must_use]
    #[inline]
    pub fn bounded<const N: usize, T>() -> (Sender<N, T>, Receiver<N, T>) {
        Ring::new()
    }
}
