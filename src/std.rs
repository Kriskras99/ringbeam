//! Re-exports of `core`/`alloc`/`loom`/`shuttle` depending on enabled features.
//!
//! For concurrency testing with [Loom](https://docs.rs/loom/latest/loom/) and [Shuttle](https://docs.rs/shuttle/latest/shuttle/)
//! various types need to be replaced with types that check the model. Doing this in every file
//! becomes messy, so we mimic the standard library here for the types that can be replaced.
//!
//! It's also used to conditionally use the `cold_path` hint which is currently unstable, so if the
//! feature `cold_path` is not enabled it's just an empty function.
//!
//! Finally, there is an alternative `MaybeUninit` type which does track if the inner type is
//! initialized. This allows tests to catch more problems and is not intended to be enabled by
//! downstream crates.

/// Memory allocation APIs.
pub mod alloc {
    extern crate alloc;
    pub use alloc::alloc::handle_alloc_error;
    #[cfg(not(feature = "_loom"))]
    pub use alloc::alloc::{alloc, dealloc};
    pub use core::alloc::Layout;
    #[cfg(feature = "_loom")]
    pub use loom::alloc::{alloc, dealloc};
}

/// Shareable mutable containers.
pub mod cell {
    #[cfg(feature = "_loom")]
    pub use loom::cell::UnsafeCell;
    #[cfg(not(feature = "_loom"))]
    pub use unsafe_cell_wrapper::UnsafeCell;

    /// A custom [`UnsafeCell`] that mimics `loom`s API.
    #[cfg(not(feature = "_loom"))]
    mod unsafe_cell_wrapper {
        #[derive(Debug)]
        #[repr(transparent)]
        /// The core primitive for interior mutability in Rust.
        pub struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

        impl<T> UnsafeCell<T> {
            /// Constructs a new instance of `UnsafeCell` which will wrap the specified value.
            ///
            /// All access to the inner value through `&UnsafeCell<T>` requires unsafe code.
            pub const fn new(data: T) -> Self {
                Self(core::cell::UnsafeCell::new(data))
            }

            /// Get a mutable pointer to the wrapped value.
            pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
                f(self.0.get())
            }
        }
    }
}

/// Hints to compiler that affects how code should be emitted or optimized.
///
/// Hints may be compile time or runtime.
pub mod hint {
    #[cfg(not(any(feature = "_loom", feature = "_shuttle")))]
    pub use core::hint::spin_loop;
    #[cfg(feature = "_loom")]
    pub use loom::hint::spin_loop;
    #[cfg(feature = "_shuttle")]
    pub use shuttle::hint::spin_loop;

    #[cfg(not(feature = "cold_path"))]
    /// Does nothing as the `cold_path` feature is not enabled.
    pub const fn cold_path() {}
    #[cfg(feature = "cold_path")]
    pub use core::hint::cold_path;
}

/// Basic functions for dealing with memory.
pub mod mem {
    #[cfg(feature = "_safe_maybeuninit")]
    pub use safe_maybe_uninit::MaybeUninit;
    #[cfg(not(feature = "_safe_maybeuninit"))]
    pub use unsafe_maybe_uninit::MaybeUninit;
    #[cfg(feature = "_safe_maybeuninit")]
    /// An implementation for a custom [`core::mem::MaybeUninit`] that tracks the internal state.
    ///
    /// This module is not compatible with `core` and `alloc`.
    mod safe_maybe_uninit {
        use std::sync::Mutex;

        /// A `MaybeUninit` that tracks if it has been initialized.
        ///
        /// This version does *not* have the same size as T.
        pub struct MaybeUninit<T> {
            mutex: Mutex<(std::mem::MaybeUninit<T>, bool)>,
        }

        impl<T> MaybeUninit<T> {
            /// Create a new uninitialized T.
            #[must_use]
            pub const fn uninit() -> Self {
                Self {
                    mutex: Mutex::new((std::mem::MaybeUninit::uninit(), false)),
                }
            }
            /// Extract T from the container.
            ///
            /// # Panics
            /// Can panic if T is not initialized or another thread is currently writing to it.
            ///
            /// # Safety
            /// It does not have any safety requirements, the function signature just matches
            /// [`core::mem::MaybeUninit`].
            pub unsafe fn assume_init_take(&mut self) -> T {
                let mut guard = self
                    .mutex
                    .try_lock()
                    .expect("There is a concurrent access!");
                assert!(guard.1, "Container is not initialized!");
                guard.1 = false;
                let taken = std::mem::replace(&mut guard.0, std::mem::MaybeUninit::uninit());
                // SAFETY: the assert checked that it's initialized
                unsafe { taken.assume_init() }
            }

            /// Write a valid value of T.
            ///
            /// # Panics
            /// Can panic if another thread is currently reading it.
            pub fn write(&mut self, value: T) {
                let mut guard = self
                    .mutex
                    .try_lock()
                    .expect("There is a concurrent access!");
                assert!(!guard.1, "Container already initialized!");
                guard.1 = true;
                guard.0.write(value);
            }

            /// Drop T from the container.
            ///
            /// # Panics
            /// Can panic if T is not initialized or another thread is currently writing to it.
            ///
            /// # Safety
            /// It does not have any safety requirements, the function signature just matches
            /// [`core::mem::MaybeUninit`].
            pub unsafe fn assume_init_drop(&mut self) {
                let mut guard = self
                    .mutex
                    .try_lock()
                    .expect("There is a concurrent access!");
                assert!(guard.1, "Container is not initialized!");
                guard.1 = false;
                // SAFETY: The assert assures that the value is initialized
                unsafe {
                    guard.0.assume_init_drop();
                }
            }
        }
    }

    #[cfg(not(feature = "_safe_maybeuninit"))]
    mod unsafe_maybe_uninit {
        /// See [`MaybeUninit`](core::mem::MaybeUninit).
        #[repr(transparent)]
        pub struct MaybeUninit<T> {
            /// The `MaybeUninit` we're wrapping
            inner: core::mem::MaybeUninit<T>,
        }

        impl<T> MaybeUninit<T> {
            /// Create a new uninitialized T.
            #[must_use]
            pub const fn uninit() -> Self {
                Self {
                    inner: core::mem::MaybeUninit::uninit(),
                }
            }
            /// Take T from the container, replacing it with an uninitialized value.
            ///
            /// # Safety
            /// See [`MaybeUninit::assume_init`](core::mem::MaybeUninit::assume_init)
            pub const unsafe fn assume_init_take(&mut self) -> T {
                let taken = core::mem::replace(&mut self.inner, core::mem::MaybeUninit::uninit());
                // SAFETY: caller is responsible for this
                unsafe { taken.assume_init() }
            }

            /// Write a valid value of T.
            pub const fn write(&mut self, value: T) {
                self.inner.write(value);
            }

            /// Drop T from the container.
            ///
            /// # Safety
            /// See [`MaybeUninit::assume_init_drop`](core::mem::MaybeUninit::assume_init_drop)
            pub unsafe fn assume_init_drop(&mut self) {
                // SAFETY: Guaranteed by caller
                unsafe {
                    self.inner.assume_init_drop();
                }
            }
        }
    }
}

/// Synchronization primitives.
pub mod sync {
    /// Atomic types.
    pub mod atomic {
        #[cfg(not(any(feature = "_loom", feature = "_shuttle")))]
        pub use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(feature = "_loom")]
        pub use loom::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(feature = "_shuttle")]
        pub use shuttle::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
    }
}
