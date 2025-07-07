#![expect(unused, reason = "Still developing")]
mod cache_padded;
mod consumer;
mod headtail;
mod producer;
mod ring;

pub use consumer::{MultiConsumer, SingleConsumer};
pub use headtail::{HeadTail, HeadTailSync, RelaxedTailSync, TailSync};
pub use producer::Sender;
pub use ring::{Multi, Single};

use crate::consumer::Receiver;
use crate::ring::{IsMulti, Ring};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Channel is closed")]
    Closed,
    #[error("Channel is full")]
    Full,
    #[error("Channel had room, but not enough room for all items")]
    NotEnoughSpace,
}

pub type MP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Multi, R>;
pub type SP<const N: usize, T, C, R> = Sender<N, T, TailSync, C, Single, R>;
pub type MC<const N: usize, T, P, S> = MultiConsumer<N, T, TailSync, P, S>;
pub type SC<const N: usize, T, P, S> = SingleConsumer<N, T, TailSync, P, S>;

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
pub fn bounded<const N: usize, T, P, C, S, R, ReceiveHalf>()
-> (Sender<N, T, P, C, S, R>, ReceiveHalf)
where
    ReceiveHalf: Receiver<N, T, P, C, S, R>,
    P: HeadTail,
    C: HeadTail,
    S: IsMulti,
    R: IsMulti,
{
    Ring::new()
}
