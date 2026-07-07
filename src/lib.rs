//! C ABI entry point and API tables for the ithmb-core-cabi dynamic library.
//!
//! This crate compiles as a cdylib (`.so` / `.dylib` / `.dll`) that
//! implements the `ImageGlass` v10 native plugin ABI. Any language that
//! can call C functions can load this library and use it to decode .ithmb files.
//!
//! ## Public C API
//!
//! The only symbol exported by this library is:
//!
//! ```c
//! const IGPluginApi* ig_plugin_get_api(i32 abi_version);
//! ```
//!
//! Call this to obtain the plugin API table, which exposes:
//! - `codec_capability` — get decoder capability flags
//! - `codec_sniff` — test if data is a supported .ithmb format
//! - `codec_sniff_mime` — test by MIME type
//! - `codec_load_metadata` — read file dimensions without decoding pixels
//! - `codec_decode` — decode the full image
//! - `codec_decode_raster` — direct BGRA raster decode
//!
//! See the `types` module for struct definitions (`IGPluginApi`, `IGCodecApi`,
//! `IGPixelBuffer`, etc.) and the `ImageGlass` plugin SDK documentation.
//!
//! ## Usage from Python (ctypes)
//!
//! ```python
//! import ctypes
//! lib = ctypes.CDLL("./libithmb_core_cabi.so")
//! api = lib.ig_plugin_get_api(1_000_000)
//! # api contains function pointers for decoding
//! ```
//!
//! ## Usage from C / C++
//!
//! ```c
//! #include "ig_plugin.h"
//! const IGPluginApi* api = ig_plugin_get_api(IG_PLUGIN_ABI_VERSION);
//! const IGCodecApi* codec = api->get_codec_api(api, 0);
//! ```

// The usize ↔ i32 casts are required by the ImageGlass ABI (all length
// fields are `i32`).  Our strings are tiny; truncation is impossible.
// Similarly, the `i32` → `usize` casts are guarded by `len >= 0` checks.
// The `u16` → `u8` casts in ASCII-comparison helpers are safe because
// our extensions are pure ASCII.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

pub mod allocator;
pub mod buffer_registry;
pub mod logging;
pub mod types;

use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use libc::c_void;

use crate::allocator::HostAllocator;
use crate::buffer_registry::BufferRegistry;
use crate::logging::Logger;
use crate::types::{
    ig_status_from_decode_error, IGCodecApi, IGCodecCapability, IGHostApi, IGImageInfo,
    IGPixelBuffer, IGPluginApi, IGPluginInfo, IGStatus, IGStringRef,
};

use ithmb_core::decode_ithmb;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The ABI version this plugin implements (v1.0.0.0).
const IG_PLUGIN_ABI_VERSION: i32 = 1_000_000;

// .ithmb files have no fixed magic at offset 0.
// Codec selection is by extension + priority, not signature.
// ---------------------------------------------------------------------------
// Helper: encode a &str as UTF-16
// ---------------------------------------------------------------------------

fn encode_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Converts a UTF-16 [`IGStringRef`] to a [`String`] using lossy conversion.
fn utf16_to_string(s: &IGStringRef) -> Option<String> {
    if s.data.is_null() || s.length <= 0 {
        return None;
    }
    // SAFETY: caller guarantees the pointer is valid for `length` code units.
    let slice = unsafe { std::slice::from_raw_parts(s.data, s.length as usize) };
    Some(String::from_utf16_lossy(slice))
}

// ---------------------------------------------------------------------------
// Extensions array
//
// Stored as a separate static so that IGCodecCapability.extensions can
// point into it without creating a self-referencing struct (which would
// be invalidated by moves during OnceLock initialization).
// ---------------------------------------------------------------------------

/// Wrapper around the extensions pointer array for static storage.
///
/// # Safety
///
/// The referenced data is in the read-only data section and never changes
/// after initialisation.  `IGStringRef` contains `*const u16` which is
/// neither `Send` nor `Sync` by default, but the pointed-to data is
/// immutable and lives for the program lifetime.
#[repr(transparent)]
struct ExtensionsArray([IGStringRef; 2]);

// SAFETY: `ExtensionsArray` only stores pointers to const data in the
// binary's read-only section — they never dangle or mutate.
unsafe impl Send for ExtensionsArray {}
unsafe impl Sync for ExtensionsArray {}

static PLUGIN_EXTENSIONS: OnceLock<ExtensionsArray> = OnceLock::new();

// ---------------------------------------------------------------------------
// Plugin state
//
// Holds all backing string buffers and the ABI function tables.  This is
// stored in a OnceLock so that:
//   1. raw pointers into the Vec heap-buffers are stable after init, and
//   2. the plugin is lazily initialized on first access.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct PluginState {
    // UTF-16 string buffers — IGStringRef.data fields point into these
    // (heap-allocated; .as_ptr() is stable after OnceLock init).  The
    // fields are "dead" from Rust's perspective but MUST stay alive for
    // the raw pointers in `capability` / `plugin_api` to remain valid.
    plugin_id: Vec<u16>,
    plugin_name: Vec<u16>,
    plugin_version: Vec<u16>,
    cap_name: Vec<u16>,

    // ABI structs (reference the string buffers above + PLUGIN_EXTENSIONS)
    capability: IGCodecCapability,
    codec_api: IGCodecApi,
    plugin_api: IGPluginApi,
}

// SAFETY: PluginState is only stored in a OnceLock and accessed immutably
// after initialization.  All raw pointers within reference either external
// statics (PLUGIN_EXTENSIONS) or heap-allocated Vec buffers owned by the
// state itself — both of which are stable for the program lifetime.
unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

static PLUGIN_STATE: OnceLock<PluginState> = OnceLock::new();

// ---------------------------------------------------------------------------
// Host API pointer
// ---------------------------------------------------------------------------

struct HostApiPtr(*const IGHostApi);

// SAFETY: The host API pointer is stored during `initialize()` and is
// valid for the entire lifetime of the plugin.  Access is read-only after
// init.
unsafe impl Send for HostApiPtr {}
unsafe impl Sync for HostApiPtr {}

static HOST_API: OnceLock<HostApiPtr> = OnceLock::new();

/// Global registry of live pixel-buffer allocations.
static BUFFER_REGISTRY: OnceLock<BufferRegistry> = OnceLock::new();

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Ensures all plugin state is initialized.  Called by `ig_plugin_get_api`.
fn ensure_initialized() {
    // 1. Extensions array (static data — never moves, never freed).
    let _ = PLUGIN_EXTENSIONS.get_or_init(|| {
        const EXT_ITHMB_DATA: [u16; 6] = [
            b'.' as u16,
            b'i' as u16,
            b't' as u16,
            b'h' as u16,
            b'm' as u16,
            b'b' as u16,
        ];
        const EXT_IPM_DATA: [u16; 4] = [b'.' as u16, b'i' as u16, b'p' as u16, b'm' as u16];

        ExtensionsArray([
            IGStringRef {
                data: EXT_ITHMB_DATA.as_ptr(),
                length: EXT_ITHMB_DATA.len() as i32,
            },
            IGStringRef {
                data: EXT_IPM_DATA.as_ptr(),
                length: EXT_IPM_DATA.len() as i32,
            },
        ])
    });

    // 2. Plugin state (all other string buffers + ABI tables).
    let _ = PLUGIN_STATE.get_or_init(|| {
        let plugin_id = encode_utf16("ithmb-codec");
        let plugin_name = encode_utf16("iThmb Codec");
        let plugin_version = encode_utf16("1.0.3");
        let cap_name = encode_utf16("iThmb Codec");

        let capability = IGCodecCapability {
            codec_id: IGStringRef {
                data: plugin_id.as_ptr(),
                length: plugin_id.len() as i32,
            },
            name: IGStringRef {
                data: cap_name.as_ptr(),
                length: cap_name.len() as i32,
            },
            metadata_priority: 200,
            decode_priority: 200,
            supports_animation: 0,
            supports_multi_frame: 1,
            supports_cancellation: 1,
            supports_thread_safety: 1,
            extension_count: 2,
            // SAFETY: PLUGIN_EXTENSIONS is initialized directly above and
            // references const data in the binary's read-only section — the
            // pointer remains valid for the entire program lifetime.
            extensions: PLUGIN_EXTENSIONS
                .get()
                .map_or(std::ptr::null(), |e| e.0.as_ptr()),
        };

        let codec_api = IGCodecApi {
            struct_size: std::mem::size_of::<IGCodecApi>() as i32,
            abi_version: IG_PLUGIN_ABI_VERSION,
            get_capability: Some(codec_get_capability as _),
            can_handle_extension: Some(codec_can_handle_extension as _),
            can_handle_signature: Some(codec_can_handle_signature as _),
            load_metadata: Some(codec_load_metadata as _),
            decode_static_raster: Some(codec_decode_static_raster as _),
            free_pixel_buffer: Some(codec_free_pixel_buffer as _),
        };

        let plugin_api = IGPluginApi {
            struct_size: std::mem::size_of::<IGPluginApi>() as i32,
            abi_version: IG_PLUGIN_ABI_VERSION,
            info: IGPluginInfo {
                plugin_id: IGStringRef {
                    data: plugin_id.as_ptr(),
                    length: plugin_id.len() as i32,
                },
                name: IGStringRef {
                    data: plugin_name.as_ptr(),
                    length: plugin_name.len() as i32,
                },
                version: IGStringRef {
                    data: plugin_version.as_ptr(),
                    length: plugin_version.len() as i32,
                },
                abi_version: IG_PLUGIN_ABI_VERSION,
                codec_count: 1,
            },
            get_codec: Some(plugin_get_codec as _),
            initialize: Some(plugin_initialize as _),
            shutdown: Some(plugin_shutdown as _),
            self_test: Some(plugin_self_test as _),
        };

        PluginState {
            plugin_id,
            plugin_name,
            plugin_version,
            cap_name,
            capability,
            codec_api,
            plugin_api,
        }
    });
}

// ---------------------------------------------------------------------------
// Plugin API implementation
// ---------------------------------------------------------------------------

/// Returns the [`IGCodecApi`] for the codec at the given index.
///
/// We expose exactly one codec (index 0).  All other indices return null.
unsafe extern "C" fn plugin_get_codec(
    _plugin: *const IGPluginApi,
    index: i32,
) -> *const IGCodecApi {
    let result = catch_unwind(|| -> *const IGCodecApi {
        if index != 0 {
            return std::ptr::null();
        }
        PLUGIN_STATE
            .get()
            .map_or(std::ptr::null(), |s| std::ptr::from_ref(&s.codec_api))
    });

    result.unwrap_or(std::ptr::null())
}

/// Stores the host API pointer for later use by the decode dispatch (T7).
unsafe extern "C" fn plugin_initialize(
    _plugin: *const IGPluginApi,
    host: *const IGHostApi,
) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        if host.is_null() {
            return IGStatus::InvalidArg;
        }

        // Store the host API pointer so T7 (decode dispatch) can access it.
        // If set() fails, the host pointer was already stored (double init).
        if HOST_API.set(HostApiPtr(host)).is_err() {
            return IGStatus::Internal;
        }

        // Log successful initialisation.
        if let Some(host_ptr) = HOST_API.get() {
            // SAFETY: the host API pointer is valid and will remain so
            // for the lifetime of the plugin (guaranteed by ImageGlass).
            let host_api = unsafe { &*host_ptr.0 };
            if !host_api.core.is_null() {
                let logger = Logger::new(host_api.core);
                // SAFETY: Logger::info is safe to call; host_api verified non-null above.
                unsafe {
                    logger.info("ithmb-codec: initialized");
                }
            }
        }

        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

/// Shuts down the plugin.
unsafe extern "C" fn plugin_shutdown(_plugin: *const IGPluginApi) -> IGStatus {
    // The `let _ =` is deliberate: `catch_unwind` returns `Result`, but we
    // always return `Ok` regardless of whether the unwind guard fired.
    #[allow(clippy::let_unit_value)]
    let _ = catch_unwind(|| {
        if let Some(host_ptr) = HOST_API.get() {
            // SAFETY: the host pointer is still valid during shutdown.
            let host_api = unsafe { &*host_ptr.0 };
            if !host_api.core.is_null() {
                let logger = Logger::new(host_api.core);
                // SAFETY: Logger::info is safe to call; host_api verified non-null above.
                unsafe {
                    logger.info("ithmb-codec: shutdown");
                }
            }
        }
    });

    IGStatus::Ok
}

/// Trivial self-test — always passes.
unsafe extern "C" fn plugin_self_test(_plugin: *const IGPluginApi) -> IGStatus {
    IGStatus::Ok
}

// ---------------------------------------------------------------------------
// Codec API implementation
// ---------------------------------------------------------------------------
/// Returns a mutable pointer to the static [`IGCodecCapability`] struct.
unsafe extern "C" fn codec_get_capability(_codec: *const IGCodecApi) -> *mut IGCodecCapability {
    if let Some(api) = get_host_api().filter(|a| !a.core.is_null()) {
        unsafe {
            Logger::new(api.core)
                .info("ithmb-codec: get_capability called - ext_count=2, priority=200");
        }
    }
    let result = catch_unwind(|| -> *mut IGCodecCapability {
        PLUGIN_STATE.get().map_or(std::ptr::null_mut(), |s| {
            std::ptr::from_ref(&s.capability).cast_mut()
        })
    });
    result.unwrap_or(std::ptr::null_mut())
}
/// Checks whether the given file extension is supported.
///
/// Performs a case-insensitive ASCII comparison against `.ithmb` and `.ipm`.
unsafe extern "C" fn codec_can_handle_extension(
    _codec: *const IGCodecApi,
    ext: *const u16,
    len: i32,
) -> i32 {
    if let Some(api) = get_host_api().filter(|a| !a.core.is_null()) {
        let ext_str = if ext.is_null() || len <= 0 {
            String::from("null")
        } else {
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ext, len as usize) })
        };
        unsafe {
            Logger::new(api.core).info(&format!(
                "ithmb-codec: can_handle_extension('{ext_str}', len={len})"
            ));
        }
    }
    if ext.is_null() || len <= 0 {
        return 0;
    }

    let result = catch_unwind(|| -> i32 {
        let exts = match PLUGIN_EXTENSIONS.get() {
            Some(e) => &e.0,
            None => return 0,
        };

        // SAFETY: caller provides a valid pointer and length >= 1 (checked
        // above).  The host guarantees the buffer is valid for `len` code units.
        #[allow(clippy::cast_sign_loss)]
        let input_slice = unsafe { std::slice::from_raw_parts(ext, len as usize) };

        for known_ext in exts {
            if known_ext.length != len || known_ext.data.is_null() {
                continue;
            }

            // SAFETY: known_ext references static data with length matching
            // the actual const-array size — always within bounds.
            #[allow(clippy::cast_sign_loss)]
            let known_slice =
                unsafe { std::slice::from_raw_parts(known_ext.data, known_ext.length as usize) };

            // Both slices contain ASCII text only (`.`, `i`, `t`, `h`, `m`, `b`, `p`).
            let eq = input_slice
                .iter()
                .zip(known_slice.iter())
                .all(|(a, b)| (*a as u8).eq_ignore_ascii_case(&(*b as u8)));

            if eq {
                return 1;
            }
        }

        0
    });

    result.unwrap_or(0)
}

/// .ithmb files have no fixed magic signature at offset 0.
/// We rely on extension matching + decode priority for selection.
unsafe extern "C" fn codec_can_handle_signature(
    _codec: *const IGCodecApi,
    _data: *const u8,
    _len: i32,
) -> i32 {
    0
}

/// Stub — metadata loading is not implemented in this scope.
unsafe extern "C" fn codec_load_metadata(
    _codec: *const IGCodecApi,
    _path: *const IGStringRef,
    _info: *mut IGImageInfo,
) -> IGStatus {
    IGStatus::NotImplemented
}

/// Returns the global [`BufferRegistry`] instance.
fn get_buffer_registry() -> &'static BufferRegistry {
    BUFFER_REGISTRY.get_or_init(BufferRegistry::new)
}

/// Creates a [`HostAllocator`] from the stored host API, if available.
fn get_host_allocator() -> Option<HostAllocator> {
    let host_api = get_host_api()?;
    if host_api.core.is_null() {
        return None;
    }
    Some(HostAllocator::new(host_api.core))
}

unsafe extern "C" fn codec_decode_static_raster(
    _codec: *const IGCodecApi,
    path: *const IGStringRef,
    _params: *const IGStringRef,
    frame_index: i32,
    buffer: *mut IGPixelBuffer,
) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        // ---- Input validation ----
        if path.is_null() || buffer.is_null() {
            return IGStatus::InvalidArg;
        }

        let path_ref = unsafe { &*path };
        let Some(path_str) = utf16_to_string(path_ref) else {
            return IGStatus::InvalidArg;
        };

        // Only single-frame static images are supported
        if frame_index != 0 {
            return IGStatus::InvalidArg;
        }

        // ---- Read file ----
        let file_bytes = match std::fs::read(&path_str) {
            Ok(data) => data,
            Err(e) => {
                if let Some(host_api) = get_host_api().filter(|api| !api.core.is_null()) {
                    let logger = Logger::new(host_api.core);
                    // SAFETY: Logger::error is safe to call; host_api verified non-null above.
                    unsafe {
                        logger.error(&format!("ithmb-codec: failed to read file: {e}"));
                    }
                }
                return IGStatus::IoError;
            }
        };

        // ---- Set up cancellation ----
        let canceled = Arc::new(AtomicBool::new(false));
        let cancel_flag = canceled.clone();

        let monitor = get_host_api()
            .filter(|api| !api.core.is_null())
            .and_then(|api| {
                // SAFETY: core pointer is valid for the plugin lifetime
                let check_cancel = unsafe { (*api.core).is_cancellation_requested }?;
                Some(thread::spawn(move || {
                    while !cancel_flag.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(50));
                        // SAFETY: function pointer from host, call with null context
                        if unsafe { check_cancel(std::ptr::null_mut()) } != 0 {
                            cancel_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }))
            });

        // ---- Decode ----
        let decoded = match decode_ithmb(&file_bytes, &canceled) {
            Ok(img) => img,
            Err(e) => {
                canceled.store(true, Ordering::Relaxed);
                if let Some(handle) = monitor {
                    let _ = handle.join();
                }
                return ig_status_from_decode_error(&e);
            }
        };

        // Signal cancellation monitor to stop
        canceled.store(true, Ordering::Relaxed);
        if let Some(handle) = monitor {
            let _ = handle.join();
        }

        // ---- Allocate host buffer ----
        let allocator = match get_host_allocator() {
            Some(a) if !a.is_null() => a,
            _ => return IGStatus::Internal,
        };

        let width = decoded.width as i32;
        let height = decoded.height as i32;
        let stride = width * 4;
        let buf_size = (height as usize) * (stride as usize);

        let data_ptr = unsafe { allocator.alloc(buf_size).cast::<u8>() };
        if data_ptr.is_null() {
            return IGStatus::OutOfMemory;
        }

        // Copy decoded BGRA data into the host buffer
        unsafe {
            std::ptr::copy_nonoverlapping(decoded.data.as_ptr(), data_ptr, buf_size);
        }

        // ---- Register buffer ----
        let registry = get_buffer_registry();
        if registry.register(data_ptr, buf_size).is_err() {
            unsafe {
                allocator.free(data_ptr.cast::<c_void>());
            }
            return IGStatus::Internal;
        }

        // ---- Populate IGPixelBuffer ----
        unsafe {
            (*buffer).data = data_ptr;
            (*buffer).width = width;
            (*buffer).height = height;
            (*buffer).stride = stride;
            (*buffer).pixel_format = 0; // BGRA32
            (*buffer).release_context = std::ptr::null_mut();
        }

        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

unsafe extern "C" fn codec_free_pixel_buffer(
    _codec: *const IGCodecApi,
    buffer: *mut IGPixelBuffer,
) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        if buffer.is_null() {
            return IGStatus::InvalidArg;
        }

        let data_ptr = unsafe { (*buffer).data };
        if data_ptr.is_null() {
            return IGStatus::Ok;
        }

        // ---- Unregister from buffer registry ----
        let registry = get_buffer_registry();
        let Ok(_entry) = registry.unregister(data_ptr) else {
            if let Some(host_api) = get_host_api().filter(|api| !api.core.is_null()) {
                let logger = Logger::new(host_api.core);
                // SAFETY: Logger::warn is safe to call; host_api verified non-null above.
                unsafe {
                    logger.warn("ithmb-codec: free_pixel_buffer: untracked buffer");
                }
            }
            return IGStatus::InvalidArg;
        };

        // ---- Free via host allocator ----
        let allocator = match get_host_allocator() {
            Some(a) if !a.is_null() => a,
            _ => return IGStatus::Internal,
        };

        // SAFETY: allocator.free is the inverse of the allocation; pointer comes from host allocator.
        unsafe {
            allocator.free(data_ptr.cast::<c_void>());
        }

        // Clear the buffer struct fields
        unsafe {
            (*buffer).data = std::ptr::null_mut();
            (*buffer).width = 0;
            (*buffer).height = 0;
            (*buffer).stride = 0;
        }

        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Returns a reference to the stored host API, if available.
///
/// This is used by other modules (e.g., decode dispatch in T7) to access
/// host services such as memory allocation and logging.
#[must_use]
pub fn get_host_api() -> Option<&'static IGHostApi> {
    HOST_API.get().map(|ptr| {
        // SAFETY: the host API pointer was stored during `initialize()` and
        // is valid for the entire lifetime of the plugin (guaranteed by
        // ImageGlass).
        unsafe { &*ptr.0 }
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// C ABI entry point — returns the [`IGPluginApi`] function table.
///
/// This is the only public symbol exported by the cdylib.  `ImageGlass` calls
/// it to obtain the plugin's function table, which it then uses to enumerate
/// codecs, initialise the plugin, and decode files.
///
/// # Safety
///
/// The caller must pass a valid `abi_version` matching `IG_PLUGIN_ABI_VERSION`. The returned pointer is valid for the
/// lifetime of the process.
///
/// # Returns
///
/// * A pointer to the static [`IGPluginApi`] on success.
/// * `null` if `abi_version` does not match or initialisation fails.
#[unsafe(no_mangle)]
pub extern "C" fn ig_plugin_get_api(abi_version: i32) -> *const IGPluginApi {
    if abi_version != IG_PLUGIN_ABI_VERSION {
        return std::ptr::null();
    }

    let result = catch_unwind(|| -> *const IGPluginApi {
        ensure_initialized();
        PLUGIN_STATE
            .get()
            .map_or(std::ptr::null(), |s| std::ptr::from_ref(&s.plugin_api))
    });

    result.unwrap_or(std::ptr::null())
}
