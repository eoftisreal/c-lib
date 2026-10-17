//! C ABI version 1: handles and descriptor/synchronization helpers.
//!
//! No compressed-picture decoder or Vulkan Video backend is exposed yet.
//! See `docs/API.md` for ownership, threading and compatibility contracts.
//!
//! ```
//! use codec_ffi::{codec_get_abi_version, codec_get_capabilities};
//! assert_eq!(codec_get_abi_version() >> 16, 1);
//! assert_eq!(codec_get_capabilities(), 0); // No complete decoder backend.
//! ```

use codec_core::{CodecId, DecoderConfig, PidClockController};
use codec_dsl::CompiledPipelineDag;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::catch_unwind;

/// Opaque struct type in C
#[repr(C)]
pub struct DecoderHandle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CodecFrame {
    pub image_view: u64,
    pub timeline_semaphore: u64,
    pub timeline_value: u64,
    pub dpb_slot: u8,
}

#[repr(C)]
pub struct CodecPipeline {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CodecVulkanConfig {
    pub device: u64,
    pub decode_queue: u64,
    pub queue_family_index: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CodecVulkanFrameDescriptor {
    pub image_view: u64,
    pub timeline_semaphore: u64,
    pub timeline_value: u64,
    pub dpb_slot: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CodecSyncAdjustment {
    pub phase_error_sec: f64,
    pub control_signal: f64,
    pub audio_resample_ratio: f64,
}

#[repr(C)]
pub enum FfiCodecId {
    Av1 = 0,
    H264 = 1,
}

struct DecoderState {
    #[allow(dead_code)]
    id: CodecId,
    #[allow(dead_code)]
    config: Option<DecoderConfig>,
    vulkan_config: Option<CodecVulkanConfig>,
}

struct PipelineState {
    #[allow(dead_code)]
    dag: CompiledPipelineDag,
    pid: PidClockController,
}

#[repr(C)]
pub enum FfiResult {
    Ok = 0,
    InvalidData = -1,
    Unsupported = -2,
    OutOfMemory = -3,
    InternalError = -4,
    InternalPanic = -5,
}

#[no_mangle]
pub extern "C" fn codec_get_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// C ABI major in the upper 16 bits, additive minor in the lower 16 bits.
/// Major 1 freezes the documented existing symbols and layouts.
#[no_mangle]
pub extern "C" fn codec_get_abi_version() -> u32 {
    0x0001_0000
}

/// Runtime backend capabilities: bit 0 = complete AV1 pictures, bit 1 = Vulkan
/// Video decode, bit 2 = film-grain synthesis. Zero means none are available.
/// Descriptor/header helpers do not imply any of these capabilities.
#[no_mangle]
pub extern "C" fn codec_get_capabilities() -> u64 {
    0
}

#[no_mangle]
/// # Safety
/// out_handle must be non-NULL and writable; failures clear *out_handle.
pub unsafe extern "C" fn codec_decoder_create(
    codec: u32,
    out_handle: *mut *mut DecoderHandle,
) -> FfiResult {
    if out_handle.is_null() {
        return FfiResult::InvalidData;
    }

    *out_handle = std::ptr::null_mut();
    let res = catch_unwind(|| {
        let id = match codec {
            0 => CodecId::Av1,
            1 => CodecId::H264,
            _ => return FfiResult::InvalidData,
        };

        if id != CodecId::Av1 {
            return FfiResult::Unsupported;
        }

        let state = Box::new(DecoderState {
            id,
            config: None,
            vulkan_config: None,
        });

        *out_handle = Box::into_raw(state) as *mut DecoderHandle;
        FfiResult::Ok
    });

    match res {
        Ok(r) => r,
        Err(_) => FfiResult::InternalPanic,
    }
}

#[no_mangle]
/// # Safety
/// Pass NULL or one live handle returned by create, exactly once.
pub unsafe extern "C" fn codec_decoder_destroy(handle: *mut DecoderHandle) {
    let _ = catch_unwind(|| {
        if !handle.is_null() {
            let _state = Box::from_raw(handle as *mut DecoderState);
        }
    });
}

#[no_mangle]
/// # Safety
/// handle and config must be valid non-NULL pointers.
pub unsafe extern "C" fn codec_decoder_set_vulkan_context(
    handle: *mut DecoderHandle,
    config: *const CodecVulkanConfig,
) -> FfiResult {
    let res = catch_unwind(|| {
        if handle.is_null() || config.is_null() {
            return FfiResult::InvalidData;
        }
        let cfg = *config;
        if cfg.device == 0 || cfg.decode_queue == 0 {
            return FfiResult::InvalidData;
        }
        let state = &mut *(handle as *mut DecoderState);
        state.vulkan_config = Some(cfg);
        FfiResult::Ok
    });
    match res {
        Ok(r) => r,
        Err(_) => FfiResult::InternalPanic,
    }
}

#[no_mangle]
/// # Safety
/// frame and out_desc must be valid non-NULL pointers.
pub unsafe extern "C" fn codec_frame_export_vulkan_image(
    frame: *const CodecFrame,
    out_desc: *mut CodecVulkanFrameDescriptor,
) -> FfiResult {
    let res = catch_unwind(|| {
        if frame.is_null() || out_desc.is_null() {
            return FfiResult::InvalidData;
        }
        let f = &*frame;
        if f.image_view == 0 || f.dpb_slot >= 8 {
            return FfiResult::InvalidData;
        }
        *out_desc = CodecVulkanFrameDescriptor {
            image_view: f.image_view,
            timeline_semaphore: f.timeline_semaphore,
            timeline_value: f.timeline_value,
            dpb_slot: f.dpb_slot,
        };
        FfiResult::Ok
    });
    match res {
        Ok(r) => r,
        Err(_) => FfiResult::InternalPanic,
    }
}

#[no_mangle]
/// # Safety
/// dsl_source must be a valid NUL-terminated UTF-8 string and out_pipeline must be non-NULL.
pub unsafe extern "C" fn codec_pipeline_compile_dsl(
    dsl_source: *const c_char,
    out_pipeline: *mut *mut CodecPipeline,
) -> FfiResult {
    if out_pipeline.is_null() {
        return FfiResult::InvalidData;
    }
    *out_pipeline = std::ptr::null_mut();
    let res = catch_unwind(|| {
        if dsl_source.is_null() {
            return FfiResult::InvalidData;
        }
        let Ok(cstr) = CStr::from_ptr(dsl_source).to_str() else {
            return FfiResult::InvalidData;
        };
        let Ok(dag) = CompiledPipelineDag::compile_str(cstr) else {
            return FfiResult::InvalidData;
        };
        let mut pid = PidClockController::new_critically_damped();
        if dag
            .rules
            .iter()
            .any(|r| matches!(r.operator, codec_dsl::DslOperator::Interpolate { .. }))
        {
            pid.set_neural_lookahead_frames(1, 24.0);
        }
        let state = Box::new(PipelineState { dag, pid });
        *out_pipeline = Box::into_raw(state) as *mut CodecPipeline;
        FfiResult::Ok
    });
    match res {
        Ok(r) => r,
        Err(_) => FfiResult::InternalPanic,
    }
}

#[no_mangle]
/// # Safety
/// pipeline and out_adj must be valid non-NULL pointers.
pub unsafe extern "C" fn codec_pipeline_step_sync(
    pipeline: *mut CodecPipeline,
    audio_pts: f64,
    vsync_pts: f64,
    out_adj: *mut CodecSyncAdjustment,
) -> FfiResult {
    let res = catch_unwind(|| {
        if pipeline.is_null() || out_adj.is_null() {
            return FfiResult::InvalidData;
        }
        let state = &mut *(pipeline as *mut PipelineState);
        let step = state
            .pid
            .step(vsync_pts, vsync_pts, audio_pts, audio_pts, 1.0 / 60.0);
        *out_adj = CodecSyncAdjustment {
            phase_error_sec: step.phase_error_sec,
            control_signal: step.control_signal,
            audio_resample_ratio: step.audio_resample_ratio,
        };
        FfiResult::Ok
    });
    match res {
        Ok(r) => r,
        Err(_) => FfiResult::InternalPanic,
    }
}

#[no_mangle]
/// # Safety
/// Pass NULL or one live pipeline handle returned by codec_pipeline_compile_dsl, exactly once.
pub unsafe extern "C" fn codec_pipeline_destroy(pipeline: *mut CodecPipeline) {
    let _ = catch_unwind(|| {
        if !pipeline.is_null() {
            let _state = Box::from_raw(pipeline as *mut PipelineState);
        }
    });
}
