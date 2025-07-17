#![cfg_attr(feature = "trusted_len", feature(trusted_len))]
#![cfg_attr(feature = "likely", feature(cold_path))]

#[cfg(all(feature = "loom", feature = "shuttle"))]
compile_error!("Features 'loom' and 'shuttle' cannot be enabled at the same time");

mod cache_padded;
mod consumer;
pub mod modes;
mod producer;
mod ring;
mod std;

pub use consumer::Receiver;
pub use producer::Sender;
pub use ring::recv_values::RecvValues;

use crate::{
    modes::{HeadTailSync, Mode, Multi, RelaxedTailSync, Single},
    ring::Ring,
};
use thiserror::Error;

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
// TODO: WFE/SEV on ARM

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
/// - C: the sync mode of the consumer head and tail (see [`Mode`]),
pub type Mp<const N: usize, T, C> = Sender<N, T, Multi, C>;

/// A producer which can be cloned to multiple threads but only allows one producer active at a time.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - C: the sync mode of the consumer head and tail (see [`Mode`]),
pub type MpHts<const N: usize, T, C> = Sender<N, T, HeadTailSync, C>;

/// A producer which can be cloned to multiple threads where the last producer moves the tail.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - C: the sync mode of the consumer head and tail (see [`Mode`]),
pub type MpRts<const N: usize, T, C> = Sender<N, T, RelaxedTailSync, C>;

/// A producer which can only be on a single thread.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - C: the sync mode of the consumer head and tail (see [`Mode`]),
pub type Sp<const N: usize, T, C> = Sender<N, T, Single, C>;

/// A consumer which can be cloned to multiple threads.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`Mode`]),
pub type Mc<const N: usize, T, P> = Receiver<N, T, P, Multi>;

/// A consumer which can be cloned to multiple threads but only allows one consumer active at a time.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`Mode`]),
pub type McHts<const N: usize, T, P> = Receiver<N, T, P, HeadTailSync>;

/// A consumer which can be cloned to multiple threads where the last consumer moves the tail.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`Mode`]),
pub type McRts<const N: usize, T, P> = Receiver<N, T, P, RelaxedTailSync>;

/// A consumer which can only be on a single thread.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`Mode`]),
pub type Sc<const N: usize, T, P> = Receiver<N, T, P, Single>;

/// Create a multi-producer/multi-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn mpmc<const N: usize, T>() -> (Mp<N, T, Multi>, Mc<N, T, Multi>) {
    Ring::new()
}

/// Create a multi-producer/multi-consumer (HTS) channel with space for `N` values of `T`.
#[must_use]
pub fn mpmc_hts<const N: usize, T>() -> (MpHts<N, T, HeadTailSync>, McHts<N, T, HeadTailSync>) {
    Ring::new()
}

/// Create a multi-producer/multi-consumer (RTS) channel with space for `N` values of `T`.
#[must_use]
pub fn mpmc_rts<const N: usize, T>() -> (MpRts<N, T, RelaxedTailSync>, McRts<N, T, RelaxedTailSync>)
{
    Ring::new()
}

/// Create a multi-producer/single-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn mpsc<const N: usize, T>() -> (Mp<N, T, Single>, Sc<N, T, Multi>) {
    Ring::new()
}

/// Create a single-producer/multi-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn spmc<const N: usize, T>() -> (Sp<N, T, Multi>, Mc<N, T, Single>) {
    Ring::new()
}

/// Create a single-producer/single-consumer channel with space for `N` values of `T`.
#[must_use]
pub fn spsc<const N: usize, T>() -> (Sp<N, T, Single>, Sc<N, T, Single>) {
    Ring::new()
}

/// Create a custom channel with space for `N` values of `T`.
///
/// # Type parameters
/// - N: the size of the channel,
/// - T: the type that will be sent over the channel,
/// - P: the sync mode of the producer head and tail (see [`Mode`]),
/// - C: the sync mode of the consumer head and tail (see [`Mode`]),
#[must_use]
pub fn bounded<const N: usize, T, P, C>() -> (Sender<N, T, P, C>, Receiver<N, T, P, C>)
where
    P: Mode,
    C: Mode,
{
    Ring::new()
}
