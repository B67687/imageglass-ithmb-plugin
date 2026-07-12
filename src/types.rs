//! C ABI type definitions for the `ImageGlass` v10 native codec plugin interface.
//!
//! These types mirror the `ImageGlass.Codec.NativeAbi` C# structs with
//! `#[repr(C)]` layout for direct FFI.  Every type is `#[repr(C)]` and
//! derives `Debug + Clone + Copy` — the whole set is plain-old-data from
//! Rust's perspective.

use libc::c_void;

use ithmb_core::DecodeError;

// ---------------------------------------------------------------------------
// IGStatus
// ---------------------------------------------------------------------------

/// Result codes returned by all plugin API functions.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IGStatus {
    Ok = 0,
    Unsupported = 1,
    Canceled = 2,
    InvalidArg = 3,
    DecodeFailed = 4,
    OutOfMemory = 5,
    Internal = 6,
    NotImplemented = 7,
    IoError = 8,
}

// ---------------------------------------------------------------------------
// IGStringRef
// ---------------------------------------------------------------------------

/// A UTF-16 string reference used throughout the `ImageGlass` ABI.
///
/// # Safety
///
/// `data` must point to a valid UTF-16 buffer with at least `length` code
/// units.  The buffer is owned by the producer and must not be freed by
/// the consumer unless ownership has been explicitly transferred.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGStringRef {
    pub data: *const u16,
    pub length: i32,
}

// ---------------------------------------------------------------------------
// IGPixelBuffer
// ---------------------------------------------------------------------------

/// A decoded pixel buffer with metadata.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGPixelBuffer {
    pub data: *mut u8,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub pixel_format: i32,
    pub release_context: *mut c_void,
}

// ---------------------------------------------------------------------------
// IGImageInfo
// ---------------------------------------------------------------------------

/// Metadata describing a decoded image.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGImageInfo {
    pub width: i32,
    pub height: i32,
    pub pixel_format: i32,
    pub has_alpha: i32,
    pub hdr_transfer_fn: i32,
    pub color_space: i32,
    pub orientation: i32,
    pub frame_count: i32,
    pub file_size_bytes: i64,
    pub icc_profile_data: *mut u8,
    pub icc_profile_size: i32,
}

// ---------------------------------------------------------------------------
// IGRect
// ---------------------------------------------------------------------------

/// A rectangle used by `IGAnimationInfo`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

// ---------------------------------------------------------------------------
// IGAnimationInfo
// ---------------------------------------------------------------------------

/// Animation metadata for multi-frame codecs.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGAnimationInfo {
    pub frame_count: i32,
    pub loop_count: i32,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub background_color: i32,
    pub duration_100ns: *mut i64,
    pub disposal: *mut i32,
    pub frame_areas: *mut IGRect,
}

// ---------------------------------------------------------------------------
// IGCodecCapability
// ---------------------------------------------------------------------------

/// Static metadata describing a codec's capabilities.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGCodecCapability {
    pub codec_id: IGStringRef,
    pub name: IGStringRef,
    pub metadata_priority: i32,
    pub decode_priority: i32,
    pub supports_metadata: i32,
    pub supports_static_raster: i32,
    pub supports_color_profiles: i32,
    pub supports_animation: i32,
    pub extension_count: i32,
    pub extensions: *const IGStringRef,
}

// ---------------------------------------------------------------------------
// IGPluginInfo
// ---------------------------------------------------------------------------

/// Static metadata describing the plugin itself.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGPluginInfo {
    pub plugin_id: IGStringRef,
    pub name: IGStringRef,
    pub version: IGStringRef,
    pub abi_version: i32,
    pub codec_count: i32,
}

// ---------------------------------------------------------------------------
// IGHostCoreApi
// ---------------------------------------------------------------------------

/// Core host service functions provided by `ImageGlass`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGHostCoreApi {
    pub log: Option<unsafe extern "C" fn(i32, IGStringRef)>,
    pub alloc: Option<unsafe extern "C" fn(usize) -> *mut c_void>,
    pub free: Option<unsafe extern "C" fn(*mut c_void)>,
    pub is_cancellation_requested: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub get_config_directory: Option<unsafe extern "C" fn(*mut u16, i32)>,
}

// ---------------------------------------------------------------------------
// IGHostApi
// ---------------------------------------------------------------------------

/// Top-level host API provided by `ImageGlass`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGHostApi {
    pub struct_size: i32,
    pub abi_version: i32,
    pub core: *const IGHostCoreApi,
}

// ---------------------------------------------------------------------------
// IGCodecApi
// ---------------------------------------------------------------------------

/// Function table for a single codec.
///
/// Every codec exposed by a plugin provides one of these tables.  Animation
/// function pointers are included but set to `None` for static-only codecs.
///
/// # Layout (C# ABI, 9 × 8 = 72 bytes)
///
/// | Offset | Field | Type |
/// |---|---|---|
/// | 0 | `get_capability` | `fn(*mut IGCodecCapability) -> IGStatus` |
/// | 8 | `can_handle_extension` | `fn(IGStringRef) -> i32` |
/// | 16 | `can_handle_signature` | `fn(*const u8, i32) -> i32` |
/// | 24 | `load_metadata` | `fn(IGStringRef, *mut IGImageInfo, *mut c_void) -> IGStatus` |
/// | 32 | `decode_static_raster` | `fn(IGStringRef, i32, *mut IGPixelBuffer, *mut c_void) -> IGStatus` |
/// | 40 | `free_pixel_buffer` | `fn(*mut IGPixelBuffer)` |
/// | 48 | `get_animation_info` | `fn(IGStringRef, *mut IGAnimationInfo, *mut c_void) -> IGStatus` |
/// | 56 | `free_animation_info` | `fn(*mut IGAnimationInfo)` |
/// | 64 | `decode_animation_frame` | `fn(IGStringRef, i32, *mut IGPixelBuffer, *mut c_void) -> IGStatus` |
///
/// Note: unlike `IGPluginApi`, there is no `struct_size` or `abi_version` field
/// at the start — the C# `IGCodecApi` struct is pure function pointers only.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGCodecApi {
    pub get_capability: Option<unsafe extern "C" fn(*mut IGCodecCapability) -> IGStatus>,
    pub can_handle_extension: Option<unsafe extern "C" fn(IGStringRef) -> i32>,
    pub can_handle_signature: Option<unsafe extern "C" fn(*const u8, i32) -> i32>,
    pub load_metadata:
        Option<unsafe extern "C" fn(IGStringRef, *mut IGImageInfo, *mut c_void) -> IGStatus>,
    pub decode_static_raster:
        Option<unsafe extern "C" fn(IGStringRef, i32, *mut IGPixelBuffer, *mut c_void) -> IGStatus>,
    pub free_pixel_buffer: Option<unsafe extern "C" fn(*mut IGPixelBuffer)>,
    pub get_animation_info:
        Option<unsafe extern "C" fn(IGStringRef, *mut IGAnimationInfo, *mut c_void) -> IGStatus>,
    pub free_animation_info: Option<unsafe extern "C" fn(*mut IGAnimationInfo)>,
    pub decode_animation_frame:
        Option<unsafe extern "C" fn(IGStringRef, i32, *mut IGPixelBuffer, *mut c_void) -> IGStatus>,
}

// ---------------------------------------------------------------------------
// IGPluginApi
// ---------------------------------------------------------------------------

/// Function table for the plugin itself.
///
/// # Layout (C# ABI, 96 bytes)
///
/// | Offset | Field | Type |
/// |---|---|---|
/// | 0 | `struct_size` | `i32` |
/// | 4 | `abi_version` | `i32` |
/// | 8 | `info` | `IGPluginInfo` (56 bytes) |
/// | 64 | `get_codec` | `fn(i32, *mut *const IGCodecApi) -> IGStatus` |
/// | 72 | `initialize` | `fn() -> IGStatus` |
/// | 80 | `shutdown` | `fn()` |
/// | 88 | `self_test` | `fn() -> IGStatus` |
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGPluginApi {
    pub struct_size: i32,
    pub abi_version: i32,
    pub info: IGPluginInfo,
    pub get_codec: Option<unsafe extern "C" fn(i32, *mut *const IGCodecApi) -> IGStatus>,
    pub initialize: Option<unsafe extern "C" fn() -> IGStatus>,
    pub shutdown: Option<unsafe extern "C" fn()>,
    pub self_test: Option<unsafe extern "C" fn() -> IGStatus>,
}

// ---------------------------------------------------------------------------
// IGNativeAbi
// ---------------------------------------------------------------------------

/// Version stamp returned by the ABI entry point (`ig_plugin_get_api`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IGNativeAbi {
    pub ig_plugin_abi_version: i32,
}

// ===========================================================================
// Helper functions
// ===========================================================================

/// Maps an [`ithmb_core::DecodeError`] to the corresponding [`IGStatus`].
///
/// This conversion is infallible — every error variant maps to a sensible
/// status code so callers never need to handle an unmapped error.
#[must_use]
pub fn ig_status_from_decode_error(err: &DecodeError) -> IGStatus {
    match err {
        DecodeError::Io(_) => IGStatus::IoError,
        DecodeError::Jpeg(_) | DecodeError::Profile(_) => IGStatus::DecodeFailed,
        DecodeError::InvalidFormat(_) | DecodeError::BufferTooShort { .. } => IGStatus::InvalidArg,
        DecodeError::Unsupported(_) => IGStatus::Unsupported,
        DecodeError::Canceled(_) => IGStatus::Canceled,
        _ => IGStatus::Internal,
    }
}

/// Converts a `&str` to a UTF-16 `Vec<u16>` and an `IGStringRef` pointing
/// into it.
///
/// The returned `Vec<u16>` *must* outlive the `IGStringRef` — the reference
/// borrows from the vector's backing storage.  This is the standard Rust FFI
/// pattern for constructing temporary string arguments:
///
/// ```ignore
/// let (buf, ref_) = ig_string_ref_from_str("hello");
/// some_ffi_function(&ref_);   // safe as long as `buf` is still alive
/// drop(buf);                  // invalidates `ref_` — don't use it after this
/// ```
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
#[must_use]
pub fn ig_string_ref_from_str(s: &str) -> (Vec<u16>, IGStringRef) {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    // Safety: a single `&str` can never produce more than `i32::MAX` UTF-16
    // code units — that would require >4 GiB of UTF-8 input, which exceeds
    // the maximum length of a `&str` on any current platform.
    let length = utf16.len() as i32;
    let string_ref = IGStringRef {
        data: utf16.as_ptr(),
        length,
    };
    (utf16, string_ref)
}

/// Returns a null `IGStringRef` (empty string with null data pointer).
///
/// This is used to represent absent or optional string values across the FFI
/// boundary.
#[must_use]
pub fn ig_string_ref_null() -> IGStringRef {
    IGStringRef {
        data: std::ptr::null(),
        length: 0,
    }
}
