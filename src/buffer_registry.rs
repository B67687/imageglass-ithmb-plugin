//! Thread-safe registry for tracking live [`IGPixelBuffer`](crate::types::IGPixelBuffer)
//! allocations.
//!
//! The registry prevents double-free bugs and detects dangling-pointer
//! scenarios by keeping a mutex-protected map of every buffer that the
//! plugin has handed out to the host.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::types::IGStatus;

// ---------------------------------------------------------------------------
// BufferEntry
// ---------------------------------------------------------------------------

/// Metadata for a single tracked pixel-buffer allocation.
#[derive(Debug, Clone, PartialEq)]
pub struct BufferEntry {
    /// Pointer to the beginning of the pixel-data allocation.
    pub ptr: *mut u8,
    /// Allocated size in bytes.
    pub size: usize,
    /// Whether this buffer has already been freed.
    pub freed: bool,
}

// Safety: `BufferEntry` only stores a raw address as a numeric value — it
// never dereferences the pointer.  It is safe to transfer between threads.
unsafe impl Send for BufferEntry {}

// ---------------------------------------------------------------------------
// BufferRegistry
// ---------------------------------------------------------------------------

/// Thread-safe registry of live pixel buffers.
///
/// Every buffer allocated for the host is registered here so that
/// [`free_pixel_buffer`](crate::types::IGCodecApi::free_pixel_buffer) can
/// validate the pointer before passing it to the host allocator, preventing
/// double-free and use-after-free.
pub struct BufferRegistry {
    entries: Mutex<HashMap<*mut u8, BufferEntry>>,
}

// Safety: the `Mutex` provides exclusive access to the map, and the raw
// pointers stored as keys are never dereferenced — they are opaque
// identifiers only.
unsafe impl Send for BufferRegistry {}
unsafe impl Sync for BufferRegistry {}

impl BufferRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Registers a pixel-buffer allocation.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mutex is poisoned (a panic occurred while
    /// the lock was held).
    ///
    /// # Errors
    ///
    /// Returns [`IGStatus::InvalidArg`] when the pointer is already tracked
    /// (potential double-register bug).
    pub fn register(&self, data: *mut u8, size: usize) -> Result<(), IGStatus> {
        let mut map = self.entries.lock().expect("BufferRegistry lock poisoned");
        if map.contains_key(&data) {
            return Err(IGStatus::InvalidArg);
        }
        map.insert(
            data,
            BufferEntry {
                ptr: data,
                size,
                freed: false,
            },
        );
        Ok(())
    }

    /// Unregisters a buffer and returns its entry.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mutex is poisoned (a panic occurred while
    /// the lock was held).
    ///
    /// # Errors
    ///
    /// Returns [`IGStatus::InvalidArg`] when the pointer is not tracked.
    pub fn unregister(&self, data: *mut u8) -> Result<BufferEntry, IGStatus> {
        let mut map = self.entries.lock().expect("BufferRegistry lock poisoned");
        map.remove(&data).ok_or(IGStatus::InvalidArg)
    }

    /// Returns `true` when the pointer is currently tracked.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mutex is poisoned (a panic occurred while
    /// the lock was held).
    #[must_use]
    pub fn contains(&self, data: *mut u8) -> bool {
        let map = self.entries.lock().expect("BufferRegistry lock poisoned");
        map.contains_key(&data)
    }

    /// Returns the number of tracked buffers.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mutex is poisoned (a panic occurred while
    /// the lock was held).
    #[must_use]
    pub fn len(&self) -> usize {
        let map = self.entries.lock().expect("BufferRegistry lock poisoned");
        map.len()
    }

    /// Returns `true` when no buffers are tracked.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mutex is poisoned (a panic occurred while
    /// the lock was held).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let map = self.entries.lock().expect("BufferRegistry lock poisoned");
        map.is_empty()
    }
}

impl Default for BufferRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn register_and_contains() {
        let reg = BufferRegistry::new();
        let data = 0x42 as *mut u8;

        assert!(!reg.contains(data));
        assert!(reg.register(data, 128).is_ok());
        assert!(reg.contains(data));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn register_double_is_error() {
        let reg = BufferRegistry::new();
        let data = 0x42 as *mut u8;

        assert!(reg.register(data, 64).is_ok());
        assert_eq!(reg.register(data, 64), Err(IGStatus::InvalidArg));
    }

    #[test]
    fn unregister_removes_entry() {
        let reg = BufferRegistry::new();
        let data = 0x42 as *mut u8;

        reg.register(data, 256).unwrap();
        let entry = reg.unregister(data).unwrap();
        assert_eq!(entry.ptr, data);
        assert_eq!(entry.size, 256);
        assert!(!entry.freed);
        assert!(!reg.contains(data));
        assert!(reg.is_empty());
    }

    #[test]
    fn unregister_unknown_is_error() {
        let reg = BufferRegistry::new();
        assert_eq!(reg.unregister(0xDEAD as *mut u8), Err(IGStatus::InvalidArg));
    }

    #[test]
    fn empty_registry() {
        let reg = BufferRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn multiple_entries() {
        let reg = BufferRegistry::new();
        let a = 0x10 as *mut u8;
        let b = 0x20 as *mut u8;
        let c = 0x30 as *mut u8;

        reg.register(a, 16).unwrap();
        reg.register(b, 32).unwrap();
        reg.register(c, 64).unwrap();
        assert_eq!(reg.len(), 3);

        assert!(reg.contains(a));
        assert!(reg.contains(b));
        assert!(reg.contains(c));

        reg.unregister(b).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(!reg.contains(b));
        assert!(reg.contains(a));
        assert!(reg.contains(c));
    }

    #[test]
    fn null_pointer_is_valid_key() {
        let reg = BufferRegistry::new();
        assert!(reg.register(ptr::null_mut(), 0).is_ok());
        assert!(reg.contains(ptr::null_mut()));
        assert_eq!(reg.len(), 1);

        let entry = reg.unregister(ptr::null_mut()).unwrap();
        assert!(entry.ptr.is_null());
        assert_eq!(entry.size, 0);
    }
}
