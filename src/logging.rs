//! Logging wrapper around [`IGHostCoreApi::log`].
//!
//! Provides a convenient, infallible interface for sending log messages
//! from a codec plugin back to the `ImageGlass` host application.
//!
//! # Safety
//!
//! Every method on [`Logger`] is `unsafe` because it may dereference the
//! host API pointer and call the host function pointer stored therein.
//! The caller *must* guarantee that the [`IGHostCoreApi`] struct the
//! `Logger` was constructed with outlives every call — which in practice
//! means the host-provided API table is valid for the entire lifetime of
//! the plugin.

use crate::types::{ig_string_ref_from_str, IGHostCoreApi};

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

/// Severity level for log messages.
///
/// The discriminant matches `ImageGlass`'s native `LogLevel` enum so it can
/// be passed directly across the FFI boundary.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info = 0,
    Warning = 1,
    Error = 2,
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// A safe(ish) wrapper around the host's logging function.
///
/// # Panics
///
/// No method on `Logger` will panic.  If the host pointer is null or the
/// `log` field is `None` the call is silently ignored.
///
/// # Examples
///
/// ```ignore
/// let logger = Logger::new(host_api.core);
/// // SAFETY: `host_api` is valid and alive for the duration of the call.
/// unsafe { logger.info("plugin initialised"); }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Logger {
    host: *const IGHostCoreApi,
}

impl Logger {
    /// Wraps a raw pointer to the host core API table.
    ///
    /// The pointer may be null — every method will safely be a no-op in
    /// that case.
    #[must_use]
    pub fn new(host: *const IGHostCoreApi) -> Self {
        Self { host }
    }

    /// Returns `true` when the wrapped pointer is null (i.e. no host API
    /// was provided).
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.host.is_null()
    }

    /// Sends a log message at the given severity level.
    ///
    /// # Safety
    ///
    /// The caller must ensure the [`IGHostCoreApi`] pointer this logger
    /// was constructed from is still valid (not dangling, not
    /// deallocated).
    pub unsafe fn log(&self, level: LogLevel, message: &str) {
        if self.host.is_null() {
            return;
        }

        // Safety: the caller promises the pointer is valid.
        let Some(log_fn) = (unsafe { (*self.host).log }) else {
            return;
        };

        let (_utf16_buf, string_ref) = ig_string_ref_from_str(message);

        // Safety: the caller promises the host API is alive.  `string_ref`
        // borrows from `utf16_buf` which lives for the duration of this call.
        unsafe {
            log_fn(level as i32, string_ref);
        }

        // `utf16_buf` is dropped here — after `log_fn` returns.
    }

    /// Convenience: send an info-level message.
    ///
    /// # Safety
    ///
    /// Same safety contract as [`Logger::log`].
    pub unsafe fn info(&self, message: &str) {
        // Safety: deferred to the caller.
        unsafe { self.log(LogLevel::Info, message) }
    }

    /// Convenience: send a warning-level message.
    ///
    /// # Safety
    ///
    /// Same safety contract as [`Logger::log`].
    pub unsafe fn warn(&self, message: &str) {
        // Safety: deferred to the caller.
        unsafe { self.log(LogLevel::Warning, message) }
    }

    /// Convenience: send an error-level message.
    ///
    /// # Safety
    ///
    /// Same safety contract as [`Logger::log`].
    pub unsafe fn error(&self, message: &str) {
        // Safety: deferred to the caller.
        unsafe { self.log(LogLevel::Error, message) }
    }
}
