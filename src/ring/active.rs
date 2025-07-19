//! Logic for tracking the amount of consumers and producers.
use crate::{
    Error,
    std::{
        hint::cold_path,
        sync::atomic::{
            AtomicU32, Ordering,
            Ordering::{Relaxed, SeqCst},
        },
    },
};

/// A counter of active consumers and producers that can be shared between threads.
///
/// This is a wrapper around [`AtomicU32`], converting to and from [`Active`] on load and store.
pub struct AtomicActive {
    /// The encoded form of [`Active`].
    inner: AtomicU32,
}

/// Before unregistering was the entity the last in its category or the entire ring.
#[expect(clippy::enum_variant_names, reason = "Clearer this way")]
pub enum Last {
    /// There are still active entities in the category.
    NotLast,
    /// This was the last entity in the category.
    ///
    /// The tail of the category should be marked as finished.
    InCategory,
    /// This was the last entity of the ring.
    ///
    /// The ring should be cleaned up.
    InRing,
}

impl AtomicActive {
    /// Create a new counter with the given initial values.
    #[inline]
    pub fn new(consumers: u16, producers: u16) -> Self {
        Self {
            inner: AtomicU32::new(Active::new(consumers, producers).into()),
        }
    }

    /// Loads the [`Active`] value atomically.
    ///
    /// See [`AtomicU32::load`].
    #[inline]
    pub fn load(&self, ordering: Ordering) -> Active {
        self.inner.load(ordering).into()
    }

    /// Update the [`Active`] value atomically.
    ///
    /// `f` can run multiple times but it is guaranteed that the result is only stored once.
    ///
    /// See [`AtomicU32::fetch_update`].
    #[inline]
    #[expect(clippy::missing_errors_doc, reason = "Not really an error")]
    pub fn fetch_update<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        mut f: F,
    ) -> Result<Active, Active>
    where
        F: FnMut(Active) -> Option<Active>,
    {
        self.inner
            .fetch_update(set_order, fetch_order, |v| f(v.into()).map(u32::from))
            .map(Active::from)
            .map_err(Active::from)
    }

    /// Register a producer.
    ///
    /// # Errors
    /// Returns [`Error::Closed`] if the ring is closed, [`Error::Poisoned`] if the ring is in
    /// a poisoned state, [`Error::TooManyProducers`] if the maximum amount of producers is reached.
    pub fn register_producer(&self) -> Result<(), Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.producers > 0 && a.producers < u16::MAX {
                a.producers += 1;
                Some(a)
            } else {
                cold_path();
                None
            }
        })
        .map(|_old| ())
        .map_err(|old| {
            cold_path();
            match old.producers {
                0 => Error::Closed,
                u16::MAX => Error::Poisoned,
                _ => Error::TooManyProducers,
            }
        })
    }

    /// Register a consumer.
    ///
    /// # Errors
    /// Returns [`Error::Closed`] if the ring is closed, [`Error::Poisoned`] if the ring is in
    /// a poisoned state, [`Error::TooManyConsumers`] if the maximum amount of consumers is reached.
    pub fn register_consumer(&self) -> Result<(), Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.consumers > 0 && a.consumers < u16::MAX {
                a.consumers += 1;
                Some(a)
            } else {
                cold_path();
                None
            }
        })
        .map(|_old| ())
        .map_err(|old| {
            cold_path();
            match old.consumers {
                0 => Error::Closed,
                u16::MAX => Error::Poisoned,
                _ => Error::TooManyConsumers,
            }
        })
    }

    /// Unregister an active producer, returns a [`Last`] to indicate if it was the last entity.
    ///
    /// # Errors
    /// Returns [`Error::Poisoned`] if the ring is poisoned.
    ///
    /// # Panics
    /// Can panic if producers is already 0.
    pub fn unregister_producer(&self) -> Result<Last, Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.producers > 0 && a.producers < u16::MAX {
                a.producers -= 1;
                Some(a)
            } else {
                cold_path();
                None
            }
        })
        .map(|a| {
            // If the previous value had one producer remaining, that is now zero.
            match (a.producers, a.consumers) {
                (1, 0) => {
                    cold_path();
                    Last::InRing
                }
                (1, _) => {
                    cold_path();
                    Last::InCategory
                }
                (_, _) => Last::NotLast,
            }
        })
        .map_err(|a| {
            cold_path();
            if a.producers == 0 {
                cold_path();
                panic!("Producers was already 0 when trying to unregister a producer");
            } else {
                Error::Poisoned
            }
        })
    }

    /// Unregister an active consumer, returns a [`Last`] to indicate if it was the last entity.
    ///
    /// # Errors
    /// Returns [`Error::Poisoned`] if the ring is poisoned.
    ///
    /// # Panics
    /// Can panic if consumers is already 0.
    pub fn unregister_consumer(&self) -> Result<Last, Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.consumers > 0 && a.consumers < u16::MAX {
                a.consumers -= 1;
                Some(a)
            } else {
                cold_path();
                None
            }
        })
        .map(|a| {
            // If the previous value had one consumer remaining, that is now zero.
            match (a.consumers, a.producers) {
                (1, 0) => {
                    cold_path();
                    Last::InRing
                }
                (1, _) => {
                    cold_path();
                    Last::InCategory
                }
                (_, _) => Last::NotLast,
            }
        })
        .map_err(|a| {
            cold_path();
            if a.consumers == 0 {
                cold_path();
                panic!("Consumers was already 0 when trying to unregister a consumer");
            } else {
                Error::Poisoned
            }
        })
    }

    /// The amount of active producers.
    ///
    /// # Errors
    /// Can return [`Error::Poisoned`] if the ring is poisoned.
    #[inline]
    pub fn producers(&self) -> Result<u16, Error> {
        // TODO: This ordering is most likely too strict
        let producers = self.load(SeqCst).producers;
        if producers == u16::MAX {
            Err(Error::Poisoned)
        } else {
            Ok(producers)
        }
    }

    /// The amount of active consumers.
    ///
    /// # Errors
    /// Can return [`Error::Poisoned`] if the ring is poisoned.
    #[inline]
    pub fn consumers(&self) -> Result<u16, Error> {
        // TODO: This ordering is most likely too strict
        let consumers = self.load(SeqCst).consumers;
        if consumers == u16::MAX {
            Err(Error::Poisoned)
        } else {
            Ok(consumers)
        }
    }

    /// Poison the counter.
    ///
    /// This is a safe function as it will only result in a memory leak, which is safe.
    #[inline]
    pub fn poison(&self) {
        self.inner.store(u32::MAX, Relaxed);
    }

    /// Is the counter poisoned.
    ///
    /// This is a safe function as it will only result in a memory leak, which is safe.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.inner.load(Relaxed) == u32::MAX
    }
}

/// A counter of active consumers and producers.
pub struct Active {
    /// Amount of active consumers.
    ///
    /// Is `0xFFFF` if the ring is poisoned.
    pub consumers: u16,
    /// Amount of active producers.
    ///
    /// Is `0xFFFF` if the ring is poisoned.
    pub producers: u16,
}

impl Active {
    /// Create a new [`Active`] counter with the initial value.
    #[inline]
    pub const fn new(consumers: u16, producers: u16) -> Self {
        Self {
            consumers,
            producers,
        }
    }

    /// Have all producers and consumers shutdown.
    ///
    /// # Errors
    /// Can return [`Error::Poisoned`] if the ring is poisoned.
    #[inline]
    pub const fn is_empty(&self) -> Result<bool, Error> {
        if self.consumers == 0xFFFF && self.producers == 0xFFFF {
            Err(Error::Poisoned)
        } else {
            Ok(self.consumers == 0 && self.producers == 0)
        }
    }
}

impl From<u32> for Active {
    #[inline]
    fn from(value: u32) -> Self {
        let consumers = (value >> 16) as u16;
        let producers = (value & 0xFFFF) as u16;
        Self {
            consumers,
            producers,
        }
    }
}

impl From<Active> for u32 {
    #[expect(clippy::use_self, reason = "Clearer this way")]
    #[inline]
    fn from(active: Active) -> Self {
        ((active.consumers as u32) << 16) | (active.producers as u32)
    }
}
