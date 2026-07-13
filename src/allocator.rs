//! Thin wrapper around `libc::malloc`/`libc::free` for plugin-managed pixel buffers.
//!
//! The `ImageGlass` SDK rule is: **whoever allocates, frees**. Since we allocate
//! pixel buffers ourselves (via `malloc`), we must also free them in
//! `free_pixel_buffer`. We do NOT use the host allocator for pixel buffers —
//! the host provides it for host-internal use, and calling it during shutdown
//! causes crashes when the host has already partially torn down.
//!
//! Using `libc::malloc`/`free` directly keeps the plugin self-contained and
//! avoids any dependency on host lifecycle.

use libc::{c_void, free, malloc};

/// Allocate a pixel buffer of the given size.
///
/// Returns a pointer to the allocated memory, or null on allocation failure.
///
/// # Safety
///
/// The returned pointer must eventually be freed with [`pixel_buffer_free`].
#[must_use]
pub unsafe fn pixel_buffer_alloc(size: usize) -> *mut u8 {
    // SAFETY: libc::malloc is safe to call; returns null on OOM.
    unsafe { malloc(size).cast::<u8>() }
}

/// Free a pixel buffer previously allocated with [`pixel_buffer_alloc`].
///
/// # Safety
///
/// `ptr` must have been returned by [`pixel_buffer_alloc`] and not yet freed.
pub unsafe fn pixel_buffer_free(ptr: *mut u8) {
    // SAFETY: libc::free is safe to call with a pointer from malloc.
    unsafe { free(ptr.cast::<c_void>()) };
}
