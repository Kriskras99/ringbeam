#![cfg_attr(feature = "trusted_len", feature(trusted_len))]
#![cfg_attr(feature = "likely", feature(cold_path))]
#![expect(clippy::type_complexity, reason = "This crate is type generics heavy")]

#[cfg(all(feature = "loom", feature = "shuttle"))]
compile_error!("Features 'loom' and 'shuttle' cannot be enabled at the same time");

mod cache_padded;
mod consumer;
mod headtail;
mod producer;
mod ring;

pub use consumer::Receiver;
pub use headtail::{HeadTail, HeadTailSync, RelaxedTailSync, TailSync};
pub use producer::Sender;
pub use ring::{IsMulti, Multi, Single, recv_values::RecvValues};

use crate::ring::Ring;
use thiserror::Error;

#[cfg(feature = "loom")]
mod atomic {
    pub use loom::sync::atomic::{AtomicU32, Ordering, fence};
}
#[cfg(feature = "shuttle")]
mod atomic {
    pub use shuttle::sync::atomic::{AtomicU32, Ordering, fence};
}
#[cfg(not(any(feature = "loom", feature = "shuttle")))]
mod atomic {
    pub use std::sync::atomic::{AtomicU32, Ordering, fence};
}

#[cfg(feature = "loom")]
mod alloc {
    pub use loom::alloc::{Layout, alloc, dealloc};
}
#[cfg(not(feature = "loom"))]
mod alloc {
    pub use std::alloc::{Layout, alloc, dealloc};
}

#[cfg(feature = "loom")]
mod cell {
    pub use loom::cell::UnsafeCell;
}
#[cfg(not(feature = "loom"))]
mod cell {
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        pub const fn new(data: T) -> Self {
            Self(std::cell::UnsafeCell::new(data))
        }
        pub fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
            f(self.0.get())
        }

        pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
            f(self.0.get())
        }
    }
}

mod hint {
    #[cfg(feature = "loom")]
    pub use loom::hint::spin_loop;
    #[cfg(feature = "shuttle")]
    pub use shuttle::hint::spin_loop;
    #[cfg(not(any(feature = "loom", feature = "shuttle")))]
    pub use std::hint::spin_loop;

    #[cfg(not(feature = "likely"))]
    pub const fn cold_path() {}
    #[cfg(feature = "likely")]
    pub use std::hint::cold_path;
}

mod sealed {
    pub trait Sealed {}
}

// TODO: Use consistent naming for producer/consumer or sender/receiver throughout.
// TODO: Use consistent naming for enqueue/dequeue or send/recv throughout.
// TODO: Merge the HeadTail and IsMulti types/traits as the various HeadTail variations are only
//       relevant in multi scenarios.
// TODO: Implement peek for single/multi_hts
// TODO: Make testing with loom and shuttle actually work
// TODO: Maybe repr(c) on Ring, take an extra look at cache alignment.

#[derive(Debug, Error)]
pub enum Error {
    #[error("Channel is closed")]
    Closed,
    #[error("Channel is poisoned")]
    Poisoned,
    #[error("Maximum amount of producers in channel has been reached")]
    TooManyProducers,
    #[error("Maximum amount of consumers in channel has been reached")]
    TooManyConsumers,
    #[error("Channel is full")]
    Full,
    #[error("Channel is empty")]
    Empty,
    #[error("Channel had a few items, but not as many as requested")]
    NotEnoughItems,
    #[error("Channel had room, but not enough room for all the items")]
    NotEnoughSpace,
}

/// A producer which can be cloned to multiple threads.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - C: the sync mode of the consumer head and tail (see [`HeadTail`]),
/// - R: set the amount of consumers to one or many (see [`IsMulti`]).
pub type MP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Multi, R>;

/// A producer which can only be on a single thread.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - C: the sync mode of the consumer head and tail (see [`HeadTail`]),
/// - R: set the amount of consumers to one or many (see [`IsMulti`]).
pub type SP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Single, R>;

/// A consumer which can be cloned to multiple threads.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`HeadTail`]),
/// - S: set the amount of producers to one or many (see [`IsMulti`]),
pub type MC<const N: usize, T, P, S> = Receiver<N, T, TailSync, P, S, Multi>;

/// A consumer which can only be on a single thread.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`HeadTail`]),
/// - S: set the amount of producers to one or many (see [`IsMulti`]),
pub type SC<const N: usize, T, P, S> = Receiver<N, T, TailSync, P, S, Single>;

/// Create a multi-producer/multi-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn mpmc<const N: usize, T>() -> (MP<N, T, TailSync, Multi>, MC<N, T, TailSync, Multi>) {
    Ring::new()
}

/// Create a multi-producer/single-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn mpsc<const N: usize, T>() -> (MP<N, T, TailSync, Single>, SC<N, T, TailSync, Multi>) {
    Ring::new()
}

/// Create a single-producer/multi-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn spmc<const N: usize, T>() -> (SP<N, T, TailSync, Multi>, MC<N, T, TailSync, Single>) {
    Ring::new()
}

/// Create a single-producer/single-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn spsc<const N: usize, T>() -> (SP<N, T, TailSync, Single>, SC<N, T, TailSync, Single>) {
    Ring::new()
}

/// Configure a channel with space for `N` values of `T`.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`HeadTail`]),
/// - C: the sync mode of the consumer head and tail (see [`HeadTail`]),
/// - S: set the amount of producers to one or many (see [`IsMulti`]),
/// - R: set the amount of consumers to one or many (see [`IsMulti`]).
#[must_use]
pub fn bounded<const N: usize, T, P, C, S, R>()
-> (Sender<N, T, P, C, S, R>, Receiver<N, T, P, C, S, R>)
where
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    Ring::new()
}
