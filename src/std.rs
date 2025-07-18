//! Re-exports of `core`/`alloc`/`loom`/`shuttle` depending on enabled features.
//!
//! For concurrency testing with [Loom](https://docs.rs/loom/latest/loom/) and [Shuttle](https://docs.rs/shuttle/latest/shuttle/)
//! various types need to be replaced with types that check the model. Doing this in every file
//! becomes messy, so we mimic the standard library here for the types that can be replaced.
//!
//! It's also used to conditionally use the `cold_path` hint which is currently unstable, so if the
//! feature `likely` is not enabled it's just an empty function.
//!
//! Finally, there is an alternative `MaybeUninit` type which does track if the inner type is
//! initialized. This allows tests to catch more problems and is not intended to be enabled by
//! downstream crates.

/// Memory allocation APIs.
pub mod alloc {
    extern crate alloc;
    pub use alloc::alloc::handle_alloc_error;
    #[cfg(not(feature = "loom"))]
    pub use alloc::alloc::{alloc, dealloc};
    pub use core::alloc::Layout;
    #[cfg(feature = "loom")]
    pub use loom::alloc::{alloc, dealloc};
}

/// Shareable mutable containers.
pub mod cell {
    #[cfg(feature = "loom")]
    pub use loom::cell::UnsafeCell;
    #[cfg(not(feature = "loom"))]
    pub use unsafe_cell_wrapper::UnsafeCell;

    /// A custom [`UnsafeCell`] that mimics `loom`s API.
    #[cfg(not(feature = "loom"))]
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

            /// Get a const pointer to the wrapped value.
            pub fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
                f(self.0.get())
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
    #[cfg(not(any(feature = "loom", feature = "shuttle")))]
    pub use core::hint::spin_loop;
    #[cfg(feature = "loom")]
    pub use loom::hint::spin_loop;
    #[cfg(feature = "shuttle")]
    pub use shuttle::hint::spin_loop;

    #[cfg(not(feature = "likely"))]
    /// Does nothing as the `likely` feature is not enabled.
    pub const fn cold_path() {}
    #[cfg(feature = "likely")]
    pub use core::hint::cold_path;
}

/// Basic functions for dealing with memory.
pub mod mem {
    #[cfg(not(feature = "safe_maybeuninit"))]
    pub use core::mem::MaybeUninit;
    #[cfg(feature = "safe_maybeuninit")]
    pub use maybe_uninit_wrapper::MaybeUninit;
    #[cfg(feature = "safe_maybeuninit")]
    /// An implementation for a custom [`core::mem::MaybeUninit`] that tracks the internal state.
    ///
    /// This module is not compatible with `core` and `alloc`.
    mod maybe_uninit_wrapper {
        use std::sync::RwLock;

        /// A `MaybeUninit` that tracks if it has been initialized.
        ///
        /// This version does *not* have the same size as T.
        pub struct MaybeUninit<T> {
            rw: RwLock<()>,
            inner: core::mem::MaybeUninit<T>,
            // TODO: This doesn't work properly, need a `take` function
            initialized: bool,
        }

        impl<T> MaybeUninit<T> {
            /// Create a new uninitialized T.
            #[must_use]
            pub const fn uninit() -> Self {
                Self {
                    rw: RwLock::new(()),
                    inner: std::mem::MaybeUninit::uninit(),
                    initialized: false,
                }
            }
            /// Extract T from the container.
            ///
            /// # Panics
            /// Will panic if T is not initialized or another thread is currently writing to it.
            ///
            /// # Safety
            /// It does not have any safety requirements, the function signature just matches
            /// [`core::mem::MaybeUninit`].
            pub unsafe fn assume_init(mut self) -> T {
                let _guard = self.rw.try_read().unwrap();
                assert!(self.initialized, "Container is not initialized!");
                self.initialized = false;
                unsafe { self.inner.assume_init() }
            }

            /// Write a valid value of T.
            ///
            /// # Panics
            /// Will panic if another thread is currently reading it.
            pub fn write(&mut self, value: T) {
                let _guard = self.rw.try_write().unwrap();
                self.inner.write(value);
                if self.initialized {
                    eprintln!("Warning! Was already initialized!");
                }
                self.initialized = true;
            }

            /// Drop T from the container.
            ///
            /// # Panics
            /// Will panic if T is not initialized or another thread is currently writing to it.
            ///
            /// # Safety
            /// It does not have any safety requirements, the function signature just matches
            /// [`core::mem::MaybeUninit`].
            pub unsafe fn assume_init_drop(&mut self) {
                let _guard = self.rw.try_write().unwrap();
                assert!(self.initialized, "Container is not initialized!");
                self.initialized = false;
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
        #[cfg(not(any(feature = "loom", feature = "shuttle")))]
        pub use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(feature = "loom")]
        pub use loom::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(feature = "shuttle")]
        pub use shuttle::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
    }
}
