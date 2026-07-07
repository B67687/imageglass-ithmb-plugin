//! Wrapper around the host's memory-allocation functions.
//!
//! [`HostAllocator`] provides safe-ish access to the `alloc` / `free` callbacks
//! exposed through [`IGHostCoreApi`].  Callers are responsible for ensuring
//! the host API outlives the allocator and that no concurrent misuse occurs.

use crate::types::IGHostCoreApi;
use libc::c_void;

/// Thin wrapper around [`IGHostCoreApi`]'s `alloc` / `free` function pointers.
///
/// # Safety
///
/// The caller must ensure:
///
/// * The `host` pointer passed to [`new`](Self::new) points to a valid,
///   properly aligned [`IGHostCoreApi`] that remains valid for the entire
///   lifetime of this allocator.
/// * No other code calls `alloc` / `free` through the same host API while
///   this allocator is in use (the host is expected to be single-threaded
///   with respect to the plugin).
/// * Memory obtained via [`alloc`](Self::alloc) is freed exclusively through
///   [`free`](Self::free) on the **same** allocator instance.
pub struct HostAllocator {
    host: *const IGHostCoreApi,
}

impl HostAllocator {
    /// Wraps a host API pointer.
    ///
    /// # Safety
    ///
    /// `host` must be non-null, properly aligned, and valid for the entire
    /// lifetime of the returned allocator.
    #[must_use]
    pub fn new(host: *const IGHostCoreApi) -> Self {
        Self { host }
    }

    /// Allocates memory through the host allocator.
    ///
    /// Returns a pointer to the allocated block, or a null pointer if
    /// allocation fails or the host function pointer is absent.
    ///
    /// # Safety
    ///
    /// * The host API must still be alive and valid.
    /// * The returned pointer must eventually be freed with [`Self::free`].
    /// * `size` must match the actual allocation — the host may round up
    ///   internally, but the caller must not access beyond `size` bytes.
    #[must_use]
    pub unsafe fn alloc(&self, size: usize) -> *mut c_void {
        // SAFETY: The caller guarantees the host pointer is valid and
        // remains so for the duration of this call.
        unsafe {
            match (*self.host).alloc {
                Some(alloc_fn) => alloc_fn(size),
                None => std::ptr::null_mut(),
            }
        }
    }

    /// Frees memory previously allocated through this allocator.
    ///
    /// # Safety
    ///
    /// * `ptr` must have been returned by an earlier call to [`Self::alloc`]
    ///   (or a copy thereof) and must not have been freed already.
    /// * The host API must still be alive and valid.
    pub unsafe fn free(&self, ptr: *mut c_void) {
        // SAFETY: Same as [`Self::alloc`] — the caller guarantees validity.
        unsafe {
            if let Some(free_fn) = (*self.host).free {
                free_fn(ptr);
            }
        }
    }

    /// Returns `true` when the wrapped host pointer is null.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.host.is_null()
    }
}
