//! C ABI entry point and API tables for the ithmb-core-cabi dynamic library.
//!
//! This crate compiles as a cdylib (`.so` / `.dylib` / `.dll`) that
//! implements the `ImageGlass` v10 native plugin ABI.  Any language that
//! can call C functions can load this library and use it to decode .ithmb files.
//!
//! ## Public C API
//!
//! The only symbol exported by this library is:
//!
//! ```c
//! const IGPluginApi* ig_plugin_get_api(int32_t host_abi_version,
//!                                      const IGHostApi* host_api);
//! ```
//!
//! Call this to obtain the plugin API table, which exposes:
//! - `get_codec` — enumerate codecs (one static-raster codec for .ithmb)
//! - `initialize` / `shutdown` — plugin lifecycle
//! - `self_test` — trivial health check
//!
//! Each codec exposes a second function table (`IGCodecApi`) with methods for
//! capability query, extension matching, metadata loading, and raster decode.

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

use libc::c_void;

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
    // the raw pointers in `plugin_api` / capability references to remain
    // valid.
    plugin_id: Vec<u16>,
    plugin_name: Vec<u16>,
    plugin_version: Vec<u16>,
    cap_name: Vec<u16>,

    // ABI function tables (reference the string buffers above).
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

// SAFETY: The host API pointer is stored during `ig_plugin_get_api()` and is
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

        let codec_api = IGCodecApi {
            get_capability: Some(codec_get_capability as _),
            can_handle_extension: Some(codec_can_handle_extension as _),
            can_handle_signature: Some(codec_can_handle_signature as _),
            load_metadata: Some(codec_load_metadata as _),
            decode_static_raster: Some(codec_decode_static_raster as _),
            free_pixel_buffer: Some(codec_free_pixel_buffer as _),
            // Animation not supported — set all animation pointers to None.
            get_animation_info: None,
            free_animation_info: None,
            decode_animation_frame: None,
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
/// We expose exactly one codec (index 0).  All other indices write a null
/// pointer and return success.
unsafe extern "C" fn plugin_get_codec(index: i32, codec: *mut *const IGCodecApi) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        if codec.is_null() {
            return IGStatus::InvalidArg;
        }
        if index != 0 {
            unsafe {
                *codec = std::ptr::null();
            }
            return IGStatus::Ok;
        }
        let Some(state) = PLUGIN_STATE.get() else {
            return IGStatus::Internal;
        };
        unsafe {
            *codec = std::ptr::from_ref(&state.codec_api);
        }
        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

/// Plugin initialisation — the host API was already stored in the entry
/// point, so this is a no-op.
unsafe extern "C" fn plugin_initialize() -> IGStatus {
    IGStatus::Ok
}

/// Shuts down the plugin.
unsafe extern "C" fn plugin_shutdown() {
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
}

/// Trivial self-test — always passes.
unsafe extern "C" fn plugin_self_test() -> IGStatus {
    IGStatus::Ok
}

// ---------------------------------------------------------------------------
// Codec API implementation
// ---------------------------------------------------------------------------

/// Writes the codec's [`IGCodecCapability`] into the caller-provided buffer.
///
/// The capability is constructed at call time (not stored as a static), so
/// string references point into the `PluginState` string buffers and the
/// `PLUGIN_EXTENSIONS` static — all of which are stable for the program
/// lifetime.
unsafe extern "C" fn codec_get_capability(cap: *mut IGCodecCapability) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        if cap.is_null() {
            return IGStatus::InvalidArg;
        }

        let Some(state) = PLUGIN_STATE.get() else {
            return IGStatus::Internal;
        };

        let extensions_ptr = PLUGIN_EXTENSIONS
            .get()
            .map_or(std::ptr::null(), |e| e.0.as_ptr());

        unsafe {
            *cap = IGCodecCapability {
                codec_id: IGStringRef {
                    data: state.plugin_id.as_ptr(),
                    length: state.plugin_id.len() as i32,
                },
                name: IGStringRef {
                    data: state.cap_name.as_ptr(),
                    length: state.cap_name.len() as i32,
                },
                metadata_priority: 200,
                decode_priority: 200,
                supports_metadata: 1,
                supports_static_raster: 1,
                supports_color_profiles: 0,
                supports_animation: 0,
                extension_count: 2,
                extensions: extensions_ptr,
            };
        }

        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

/// Checks whether the given file extension is supported.
///
/// Performs a case-insensitive ASCII comparison against `.ithmb` and `.ipm`.
unsafe extern "C" fn codec_can_handle_extension(ext: IGStringRef) -> i32 {
    if let Some(host_api) = get_host_api().filter(|a| !a.core.is_null()) {
        let ext_str = if ext.data.is_null() || ext.length <= 0 {
            String::from("null")
        } else {
            String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(ext.data, ext.length as usize)
            })
        };
        unsafe {
            Logger::new(host_api.core)
                .info(&format!("ithmb-codec: can_handle_extension('{ext_str}')"));
        }
    }

    if ext.data.is_null() || ext.length <= 0 {
        return 0;
    }

    let result = catch_unwind(|| -> i32 {
        let exts = match PLUGIN_EXTENSIONS.get() {
            Some(e) => &e.0,
            None => return 0,
        };

        #[allow(clippy::cast_sign_loss)]
        let input_slice = unsafe { std::slice::from_raw_parts(ext.data, ext.length as usize) };

        for known_ext in exts {
            if known_ext.length != ext.length || known_ext.data.is_null() {
                continue;
            }

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
unsafe extern "C" fn codec_can_handle_signature(_data: *const u8, _len: i32) -> i32 {
    0
}

/// Reads metadata from an .ithmb file by extracting the 4-byte format prefix
/// and looking up the known dimensions from the profile database.
unsafe extern "C" fn codec_load_metadata(
    path: IGStringRef,
    info: *mut IGImageInfo,
    _cancellation: *mut c_void,
) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        if info.is_null() {
            return IGStatus::InvalidArg;
        }
        let Some(path_str) = utf16_to_string(&path) else {
            return IGStatus::InvalidArg;
        };
        let Ok(file_bytes) = std::fs::read(&path_str) else {
            return IGStatus::IoError;
        };
        if file_bytes.len() < 4 {
            return IGStatus::DecodeFailed;
        }
        let prefix =
            i32::from_be_bytes([file_bytes[0], file_bytes[1], file_bytes[2], file_bytes[3]]);
        // Fast path: try device profiles (covers common device models).
        let formats = ithmb_core::device_profiles::find_formats_by_id(prefix);
        if let Some((w, h)) = formats.iter().find_map(|f| parse_dimensions(f.description)) {
            fill_image_info(info, w, h, file_bytes.len() as i64);
            return IGStatus::Ok;
        }
        // Fallback: look up the prefix in the built-in ProfileDb (covers all 54 profiles).
        let Ok(db) = ithmb_core::profile_db::ProfileDb::load_builtin() else {
            return IGStatus::Internal;
        };
        let Some(profile) = db.get(prefix) else {
            return IGStatus::NotImplemented;
        };
        fill_image_info(
            info,
            profile.display_width() as usize,
            profile.display_height() as usize,
            file_bytes.len() as i64,
        );
        IGStatus::Ok
    });
    result.unwrap_or(IGStatus::Internal)
}

/// Helper: fill the standard IGImageInfo fields for a decoded image.
fn fill_image_info(info: *mut IGImageInfo, width: usize, height: usize, file_size: i64) {
    unsafe {
        (*info).width = width as i32;
        (*info).height = height as i32;
        (*info).pixel_format = 1; // Bgra8Unorm
        (*info).frame_count = 1;
        (*info).file_size_bytes = file_size;
    }
}

/// Returns the global [`BufferRegistry`] instance.
/// Parse a dimensions string (e.g. `"320×240"`) from a `DeviceFormatInfo` description.
fn parse_dimensions(desc: &str) -> Option<(usize, usize)> {
    // Descriptions use × (unicode multiplication sign). Height may be followed by comma/space.
    let cross = desc.find('×')?;
    let width: usize = desc[..cross].trim().parse().ok()?;
    let rest = &desc[cross + '×'.len_utf8()..];
    let height_digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let height: usize = height_digits.parse().ok()?;
    Some((width, height))
}

fn get_buffer_registry() -> &'static BufferRegistry {
    BUFFER_REGISTRY.get_or_init(BufferRegistry::new)
}

/// Decodes a static raster frame from an .ithmb file into the caller's
/// [`IGPixelBuffer`].
unsafe extern "C" fn codec_decode_static_raster(
    path: IGStringRef,
    frame_index: i32,
    buffer: *mut IGPixelBuffer,
    _cancellation: *mut c_void,
) -> IGStatus {
    let result = catch_unwind(|| -> IGStatus {
        // ---- Input validation ----
        if buffer.is_null() {
            return IGStatus::InvalidArg;
        }

        let Some(path_str) = utf16_to_string(&path) else {
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
                    unsafe {
                        logger.error(&format!("ithmb-codec: failed to read file: {e}"));
                    }
                }
                return IGStatus::IoError;
            }
        };

        // ---- Set up cancellation (poll host's cancellation inline) ----
        let canceled = Arc::new(AtomicBool::new(false));

        // ---- Decode ----
        let decoded = match decode_ithmb(&file_bytes, &canceled) {
            Ok(img) => img,
            Err(e) => {
                canceled.store(true, Ordering::Relaxed);
                return ig_status_from_decode_error(&e);
            }
        };

        // Signal cancellation monitor to stop
        canceled.store(true, Ordering::Relaxed);
        // ---- Allocate pixel buffer (self-managed) ----

        let width = decoded.width as i32;
        let height = decoded.height as i32;
        let stride = width * 4;
        let buf_size = (height as usize) * (stride as usize);

        let data_ptr = unsafe { allocator::pixel_buffer_alloc(buf_size) };
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
                allocator::pixel_buffer_free(data_ptr);
            }
            return IGStatus::Internal;
        }

        // ---- Populate IGPixelBuffer ----
        unsafe {
            (*buffer).data = data_ptr;
            (*buffer).width = width;
            (*buffer).height = height;
            (*buffer).stride = stride;
            (*buffer).pixel_format = 1; // IGPixelFormat::Bgra8Unorm
            (*buffer).release_context = std::ptr::null_mut();
        }

        IGStatus::Ok
    });

    result.unwrap_or(IGStatus::Internal)
}

unsafe extern "C" fn codec_free_pixel_buffer(buffer: *mut IGPixelBuffer) {
    #[allow(clippy::let_unit_value)]
    let _ = catch_unwind(|| {
        if buffer.is_null() {
            return;
        }

        let data_ptr = unsafe { (*buffer).data };

        // Always clear struct first — prevents ImageGlass accessing stale pointers.
        unsafe {
            (*buffer).data = std::ptr::null_mut();
            (*buffer).width = 0;
            (*buffer).height = 0;
            (*buffer).stride = 0;
        }

        if data_ptr.is_null() {
            return;
        }

        // Unregister from buffer registry
        let registry = get_buffer_registry();
        if registry.unregister(data_ptr).is_err() {
            return;
        }

        // Free via our own allocator
        unsafe {
            allocator::pixel_buffer_free(data_ptr);
        }
    });
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Returns a reference to the stored host API, if available.
///
/// This is used by other modules (e.g., the logging and allocation wrappers)
/// to access host services.
#[must_use]
pub fn get_host_api() -> Option<&'static IGHostApi> {
    HOST_API.get().map(|ptr| {
        // SAFETY: the host API pointer was stored during `ig_plugin_get_api()`
        // and is valid for the entire lifetime of the plugin (guaranteed by
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
/// # Parameters
///
/// * `host_abi_version` — the ABI version of the host (`ImageGlass`).  The major
///   version (divided by `1_000_000`) must match `IG_PLUGIN_ABI_VERSION` for
///   compatibility.
/// * `host_api` — pointer to the host API table, which provides services such
///   as logging and memory allocation.
///
/// # Safety
///
/// The caller must pass a valid `host_api` pointer that remains valid for the
/// entire lifetime of the plugin.  The returned pointer is valid for the
/// lifetime of the process.
///
/// # Returns
///
/// * A pointer to the static [`IGPluginApi`] on success.
/// * `null` if the ABI version is incompatible, `host_api` is null, or
///   initialisation fails.
#[unsafe(no_mangle)]
pub extern "C" fn ig_plugin_get_api(
    host_abi_version: i32,
    host_api: *const IGHostApi,
) -> *const IGPluginApi {
    // Check major version compatibility (e.g., 1_000_000 → major=1).
    if host_abi_version / 1_000_000 != IG_PLUGIN_ABI_VERSION / 1_000_000 {
        return std::ptr::null();
    }

    if host_api.is_null() {
        return std::ptr::null();
    }

    // Store the host API pointer so codec functions can access it later.
    // If `set()` fails, the value was already stored (identical pointer) —
    // this is not an error.
    let _ = HOST_API.set(HostApiPtr(host_api));

    let result = catch_unwind(|| -> *const IGPluginApi {
        ensure_initialized();
        PLUGIN_STATE
            .get()
            .map_or(std::ptr::null(), |s| std::ptr::from_ref(&s.plugin_api))
    });

    result.unwrap_or(std::ptr::null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dimensions() {
        assert_eq!(
            parse_dimensions("720×480 YUV422 interlaced full-screen"),
            Some((720, 480))
        );
        assert_eq!(parse_dimensions("320×240 RGB565 photo"), Some((320, 240)));
        assert_eq!(
            parse_dimensions("128×128 RGB565 cover art"),
            Some((128, 128))
        );
        assert_eq!(parse_dimensions("100×100 RGB565"), Some((100, 100)));
        assert_eq!(
            parse_dimensions("720×480 YCbCr420 padded"),
            Some((720, 480))
        );
        assert_eq!(parse_dimensions("56×56 RGB565"), Some((56, 56)));
        assert_eq!(parse_dimensions(""), None);
        // Comma-formatted descriptions (e.g. from find_formats_by_id)
        assert_eq!(
            parse_dimensions("320×320, RGB555, 204800 bytes/frame"),
            Some((320, 320))
        );
        assert_eq!(
            parse_dimensions("720×480, YCbCr420, 691200 bytes/frame"),
            Some((720, 480))
        );
    }
}
