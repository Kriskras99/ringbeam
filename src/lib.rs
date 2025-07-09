#![cfg_attr(feature = "trusted_len", feature(trusted_len))]
#![cfg_attr(feature = "likely", feature(cold_path))]
#![expect(clippy::type_complexity, reason = "This crate is type generics heavy")]
mod cache_padded;
mod consumer;
mod headtail;
mod producer;
mod ring;

pub use consumer::Receiver;
pub use headtail::{HeadTail, HeadTailSync, RelaxedTailSync, TailSync};
pub use producer::Sender;
pub use ring::{Multi, Single, recv_values::RecvValues};

use crate::ring::{IsMulti, Ring};
use thiserror::Error;

#[cfg(feature = "likely")]
use std::hint::cold_path;
#[cfg(not(feature = "likely"))]
const fn cold_path() {}

#[cfg(feature = "loom")]
mod atomic {
    pub use loom::sync::atomic::{AtomicU32, Ordering, fence};
}
#[cfg(not(feature = "loom"))]
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
    pub use std::cell::UnsafeCell;
}

#[cfg(feature = "loom")]
mod hint {
    pub use loom::hint::spin_loop;
}
#[cfg(not(feature = "loom"))]
mod hint {
    pub use std::hint::spin_loop;
}

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

pub type MP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Multi, R>;
pub type SP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Single, R>;
pub type MC<const N: usize, T, P, S> = Receiver<N, T, TailSync, P, S, Multi>;
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
