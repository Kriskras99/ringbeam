pub mod alloc {
    #[cfg(feature = "loom")]
    pub use loom::alloc::{Layout, alloc, dealloc};
    pub use std::alloc::handle_alloc_error;
    #[cfg(not(feature = "loom"))]
    pub use std::alloc::{Layout, alloc, dealloc};
}

pub mod cell {
    #[cfg(feature = "loom")]
    pub use loom::cell::UnsafeCell;
    #[cfg(not(feature = "loom"))]
    pub use unsafe_cell_wrapper::UnsafeCell;

    #[cfg(not(feature = "loom"))]
    mod unsafe_cell_wrapper {
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
}

pub mod hint {
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

pub mod mem {
    #[cfg(feature = "safe_maybeuninit")]
    pub use maybe_uninit_wrapper::MaybeUninit;
    #[cfg(not(feature = "safe_maybeuninit"))]
    pub use std::mem::MaybeUninit;
    #[cfg(feature = "safe_maybeuninit")]
    mod maybe_uninit_wrapper {
        use std::sync::RwLock;

        /// A `MaybeUninit` that tracks if it has been initialized.
        ///
        /// This version does *not* have the same size as T.
        pub struct MaybeUninit<T> {
            rw: RwLock<()>,
            inner: std::mem::MaybeUninit<T>,
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
            /// [`std::mem::MaybeUninit`].
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
            /// [`std::mem::MaybeUninit`].
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

pub mod sync {
    pub mod atomic {
        #[cfg(feature = "loom")]
        pub use loom::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(feature = "shuttle")]
        pub use shuttle::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
        #[cfg(not(any(feature = "loom", feature = "shuttle")))]
        pub use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
    }
}
