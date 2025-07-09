use crate::{
    Error,
    atomic::{
        AtomicU32, Ordering,
        Ordering::{Relaxed, SeqCst},
    },
    cold_path,
};

/// A counter of active consumers and producers that can be shared between threads.
///
/// This is a wrapper around [`AtomicU32`], converting to and from [`Active`] on load and store.
pub struct AtomicActive {
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
    pub fn new(consumers: u16, producers: u16) -> Self {
        Self {
            inner: AtomicU32::new(Active::new(consumers, producers).into()),
        }
    }

    // TODO: copy docs from atomicu32
    pub fn load(&self, ordering: Ordering) -> Active {
        self.inner.load(ordering).into()
    }

    // TODO: copy docs from atomicu32
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
            if a.producers > 0 && a.producers < (u16::MAX - 1) {
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
            if a.consumers > 0 && a.consumers < (u16::MAX - 1) {
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

    /// Unregister an active producer, return a [`Last`] to indicate if it was the last entity.
    pub fn unregister_producer(&self) -> Result<Last, Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.producers > 0 && a.producers < (u16::MAX - 1) {
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

    /// Unregister an active consumer, return a [`Last`] to indicate if it was the last entity.
    pub fn unregister_consumer(&self) -> Result<Last, Error> {
        // TODO: This ordering is most likely too strict
        self.fetch_update(SeqCst, SeqCst, |mut a| {
            if a.consumers > 0 && a.consumers < (u16::MAX - 1) {
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

    pub fn active_producers(&self) -> u16 {
        // TODO: This ordering is most likely too strict
        self.load(SeqCst).producers
    }

    pub fn active_consumers(&self) -> u16 {
        // TODO: This ordering is most likely too strict
        self.load(SeqCst).consumers
    }

    /// Poison the counter.
    ///
    /// This is a safe function as it will only result in a memory leak, which is safe.
    pub fn poison(&self) {
        self.inner.store(u32::MAX, Relaxed);
    }
}

/// A counter of active consumers and producers.
pub struct Active {
    pub consumers: u16,
    pub producers: u16,
}

impl Active {
    pub const fn new(consumers: u16, producers: u16) -> Self {
        Self {
            consumers,
            producers,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.consumers == 0 && self.producers == 0
    }
}

impl From<u32> for Active {
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
    fn from(active: Active) -> Self {
        ((active.consumers as u32) << 16) | (active.producers as u32)
    }
}
