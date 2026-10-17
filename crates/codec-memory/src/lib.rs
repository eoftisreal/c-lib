use codec_core::{CodecError, CodecResult, Frame, PixelFormat, Timestamp};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::VecDeque;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const CACHE_LINE_ALIGN: usize = 64;
pub const SUPERBLOCK_PADDING: usize = 128;
pub const FILTER_HALO_MARGIN: usize = 64;

#[derive(Debug, Clone)]
pub struct Plane {
    pub data: Arc<[u8]>,
    pub stride: usize,
    pub width: usize,
    pub height: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct CPUFrame {
    pub planes: Vec<Plane>,
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub pts: Option<Timestamp>,
}

impl Frame for CPUFrame {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        self.format
    }
    fn pts(&self) -> Option<Timestamp> {
        self.pts
    }
}

impl CPUFrame {
    pub fn new(format: PixelFormat, width: u32, height: u32) -> CodecResult<Self> {
        if width == 0 || height == 0 {
            return Err(CodecError::InvalidData);
        }
        // Bound the aggregate before any plane allocation, including on 32-bit hosts.
        let pixels = u64::from(width) * u64::from(height);
        if pixels > 128 * 1024 * 1024 {
            return Err(CodecError::OutOfMemory);
        }
        let y_stride = width as usize;
        let y_height = height as usize;

        let (planes, _) = match format {
            PixelFormat::Yuv420p => {
                let uv_stride = (width as usize).div_ceil(2);
                let uv_height = (height as usize).div_ceil(2);
                (
                    vec![
                        (y_stride, y_height),
                        (uv_stride, uv_height),
                        (uv_stride, uv_height),
                    ],
                    3,
                )
            }
            PixelFormat::Yuv422p => {
                let uv_stride = (width as usize).div_ceil(2);
                let uv_height = height as usize;
                (
                    vec![
                        (y_stride, y_height),
                        (uv_stride, uv_height),
                        (uv_stride, uv_height),
                    ],
                    3,
                )
            }
            PixelFormat::Yuv444p => {
                let uv_stride = width as usize;
                let uv_height = height as usize;
                (
                    vec![
                        (y_stride, y_height),
                        (uv_stride, uv_height),
                        (uv_stride, uv_height),
                    ],
                    3,
                )
            }
            PixelFormat::Yuv420p10 => {
                let y_stride = (width as usize) * 2;
                let uv_stride = (width as usize).div_ceil(2) * 2;
                let uv_height = (height as usize).div_ceil(2);
                (
                    vec![
                        (y_stride, y_height),
                        (uv_stride, uv_height),
                        (uv_stride, uv_height),
                    ],
                    3,
                )
            }
        };

        let bytes_per_pixel = if format == PixelFormat::Yuv420p10 {
            2
        } else {
            1
        };
        let total = planes
            .iter()
            .try_fold(0usize, |total, &(stride, height)| {
                stride
                    .checked_mul(height)
                    .and_then(|size| total.checked_add(size))
            })
            .ok_or(CodecError::OutOfMemory)?;
        if total > 128 * 1024 * 1024 {
            return Err(CodecError::OutOfMemory);
        }
        let mut plane_data = Vec::new();
        plane_data
            .try_reserve_exact(planes.len())
            .map_err(|_| CodecError::OutOfMemory)?;
        for (stride, h) in planes {
            let data_size = stride.checked_mul(h).ok_or(CodecError::OutOfMemory)?;
            // Prevent absurdly large allocations that will OOM
            if data_size > 1024 * 1024 * 128 {
                return Err(CodecError::OutOfMemory);
            }
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(data_size)
                .map_err(|_| CodecError::OutOfMemory)?;
            bytes.resize(data_size, 0);
            // Stable Rust has no fallible Arc<[u8]> conversion. Aggregate limits
            // bound this remaining allocation (which can temporarily duplicate bytes).
            let data = Arc::from(bytes.into_boxed_slice());
            plane_data.push(Plane {
                data,
                stride,
                width: stride / bytes_per_pixel,
                height: h,
                offset: 0,
            });
        }

        Ok(Self {
            planes: plane_data,
            format,
            width,
            height,
            pts: None,
        })
    }
}

/// 64-byte cache-aligned, superblock-padded (128x128) plane allocation for SIMD
/// reconstruction, 8-tap motion compensation, CDEF, and loop restoration halos.
pub struct AlignedPlaneBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
    pub visible_width: usize,
    pub visible_height: usize,
    pub padded_width: usize,
    pub padded_height: usize,
    pub stride_bytes: usize,
    pub bytes_per_sample: usize,
    pub origin_offset_bytes: usize,
}

unsafe impl Send for AlignedPlaneBuffer {}
unsafe impl Sync for AlignedPlaneBuffer {}

impl std::fmt::Debug for AlignedPlaneBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlignedPlaneBuffer")
            .field("visible_width", &self.visible_width)
            .field("visible_height", &self.visible_height)
            .field("padded_width", &self.padded_width)
            .field("padded_height", &self.padded_height)
            .field("stride_bytes", &self.stride_bytes)
            .field("bytes_per_sample", &self.bytes_per_sample)
            .finish()
    }
}

impl AlignedPlaneBuffer {
    pub fn new_superblock_padded(
        width: usize,
        height: usize,
        bytes_per_sample: usize,
    ) -> CodecResult<Self> {
        if width == 0 || height == 0 || !matches!(bytes_per_sample, 1 | 2) {
            return Err(CodecError::InvalidData);
        }
        let sb_w = width
            .div_ceil(SUPERBLOCK_PADDING)
            .checked_mul(SUPERBLOCK_PADDING)
            .ok_or(CodecError::OutOfMemory)?;
        let sb_h = height
            .div_ceil(SUPERBLOCK_PADDING)
            .checked_mul(SUPERBLOCK_PADDING)
            .ok_or(CodecError::OutOfMemory)?;
        let padded_width = sb_w
            .checked_add(2 * FILTER_HALO_MARGIN)
            .ok_or(CodecError::OutOfMemory)?;
        let padded_height = sb_h
            .checked_add(2 * FILTER_HALO_MARGIN)
            .ok_or(CodecError::OutOfMemory)?;
        let raw_stride = padded_width
            .checked_mul(bytes_per_sample)
            .ok_or(CodecError::OutOfMemory)?;
        let stride_bytes = raw_stride
            .div_ceil(CACHE_LINE_ALIGN)
            .checked_mul(CACHE_LINE_ALIGN)
            .ok_or(CodecError::OutOfMemory)?;
        let total_bytes = stride_bytes
            .checked_mul(padded_height)
            .ok_or(CodecError::OutOfMemory)?;
        if total_bytes > 128 * 1024 * 1024 {
            return Err(CodecError::OutOfMemory);
        }
        let layout = Layout::from_size_align(total_bytes, CACHE_LINE_ALIGN)
            .map_err(|_| CodecError::OutOfMemory)?;
        let raw_ptr = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw_ptr).ok_or(CodecError::OutOfMemory)?;
        let origin_offset_bytes =
            FILTER_HALO_MARGIN * stride_bytes + FILTER_HALO_MARGIN * bytes_per_sample;
        Ok(Self {
            ptr,
            layout,
            visible_width: width,
            visible_height: height,
            padded_width,
            padded_height,
            stride_bytes,
            bytes_per_sample,
            origin_offset_bytes,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.layout.size()) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }

    pub fn zero_fill(&mut self) {
        self.as_mut_slice().fill(0);
    }

    pub fn base_addr(&self) -> usize {
        self.ptr.as_ptr() as usize
    }
}

impl Clone for AlignedPlaneBuffer {
    fn clone(&self) -> Self {
        let raw_ptr = unsafe { alloc_zeroed(self.layout) };
        let ptr = NonNull::new(raw_ptr).expect("AlignedPlaneBuffer clone allocation");
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.as_ptr(), ptr.as_ptr(), self.layout.size());
        }
        Self {
            ptr,
            layout: self.layout,
            visible_width: self.visible_width,
            visible_height: self.visible_height,
            padded_width: self.padded_width,
            padded_height: self.padded_height,
            stride_bytes: self.stride_bytes,
            bytes_per_sample: self.bytes_per_sample,
            origin_offset_bytes: self.origin_offset_bytes,
        }
    }
}

impl Drop for AlignedPlaneBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

/// Typed storage backing for decoded frames: either 64-byte cache-aligned host
/// planes for software decode or zero-copy Vulkan/DMA-BUF GPU surfaces.
#[derive(Debug, Clone)]
pub enum FrameStorage {
    HostAligned(Arc<AlignedPlaneBuffer>),
    VulkanImage {
        image_view: u64,
        dpb_slot: u8,
        timeline_semaphore: u64,
        timeline_value: u64,
    },
    DmaBuf {
        fd: i32,
        drm_format_modifier: u64,
        offset: usize,
        pitch: usize,
    },
}

/// Physical slot in the Dual-Reference Virtual Buffer Index (VBI) pool.
#[derive(Debug)]
pub struct PhysicalBufferSlot {
    pub slot_index: usize,
    pub decoder_ref_count: AtomicU32,
    pub player_ref_count: AtomicU32,
    pub storage: FrameStorage,
    pub frame_id: u32,
    pub order_hint: u32,
}

impl PhysicalBufferSlot {
    pub fn dec_refs(&self) -> u32 {
        self.decoder_ref_count.load(Ordering::Acquire)
    }

    pub fn player_refs(&self) -> u32 {
        self.player_ref_count.load(Ordering::Acquire)
    }

    pub fn is_free(&self) -> bool {
        self.dec_refs() == 0 && self.player_refs() == 0
    }
}

/// Normative AV1 Dual-Reference Decoder Model (`DecoderRefCount` and `PlayerRefCount`).
/// Decouples reference frame retention (`VBI[0..8]`) from asynchronous display presentation
/// queues and `show_existing_frame` re-emissions.
#[derive(Debug)]
pub struct DualRefBufferPool {
    pub slots: Vec<PhysicalBufferSlot>,
    pub vbi: [Option<usize>; 8],
    pub presentation_queue: VecDeque<usize>,
}

impl DualRefBufferPool {
    /// Creates a pre-allocated pool of `pool_size` slots (must be >= 9, typically 16).
    pub fn new_host_pool(
        pool_size: usize,
        width: usize,
        height: usize,
        bytes_per_sample: usize,
    ) -> CodecResult<Self> {
        if pool_size < 9 {
            return Err(CodecError::InvalidData);
        }
        let mut slots = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            let buf = AlignedPlaneBuffer::new_superblock_padded(width, height, bytes_per_sample)?;
            slots.push(PhysicalBufferSlot {
                slot_index: i,
                decoder_ref_count: AtomicU32::new(0),
                player_ref_count: AtomicU32::new(0),
                storage: FrameStorage::HostAligned(Arc::new(buf)),
                frame_id: 0,
                order_hint: 0,
            });
        }
        Ok(Self {
            slots,
            vbi: [None; 8],
            presentation_queue: VecDeque::new(),
        })
    }

    /// Selects a free physical buffer slot `b_new` satisfying
    /// `DecoderRefCount[b_new] == 0 && PlayerRefCount[b_new] == 0`.
    pub fn acquire_free_slot(&self) -> CodecResult<usize> {
        self.slots
            .iter()
            .position(PhysicalBufferSlot::is_free)
            .ok_or(CodecError::OutOfMemory)
    }

    /// Atomically updates all 8 virtual slots `i in 0..8` according to `refresh_frame_flags`
    /// and optionally enqueues `b_new` for display presentation if `show_frame` is true.
    pub fn commit_decoded_frame(
        &mut self,
        b_new: usize,
        refresh_frame_flags: u8,
        show_frame: bool,
        frame_id: u32,
        order_hint: u32,
    ) -> CodecResult<()> {
        if b_new >= self.slots.len() {
            return Err(CodecError::InvalidData);
        }
        self.slots[b_new].frame_id = frame_id;
        self.slots[b_new].order_hint = order_hint;

        for i in 0..8 {
            if ((refresh_frame_flags >> i) & 1) == 1 {
                if let Some(old_b) = self.vbi[i] {
                    let prev = self.slots[old_b]
                        .decoder_ref_count
                        .fetch_sub(1, Ordering::AcqRel);
                    debug_assert!(prev >= 1);
                }
                self.vbi[i] = Some(b_new);
                self.slots[b_new]
                    .decoder_ref_count
                    .fetch_add(1, Ordering::AcqRel);
            }
        }

        if show_frame {
            self.slots[b_new]
                .player_ref_count
                .fetch_add(1, Ordering::AcqRel);
            self.presentation_queue.push_back(b_new);
        }
        Ok(())
    }

    /// Handles `show_existing_frame == 1` with `idx = frame_to_show_map_idx`:
    /// No new buffer is allocated; increments `PlayerRefCount[VBI[idx]]` and enqueues
    /// for presentation. If `refresh_all_vbi` is true (e.g. shown frame is KEY_FRAME),
    /// refreshes all 8 VBI slots (`refresh_frame_flags = 255`) to `b_existing`.
    pub fn show_existing_frame(
        &mut self,
        frame_to_show_map_idx: usize,
        refresh_all_vbi: bool,
    ) -> CodecResult<usize> {
        if frame_to_show_map_idx >= 8 {
            return Err(CodecError::InvalidData);
        }
        let b_existing = self.vbi[frame_to_show_map_idx].ok_or(CodecError::InvalidData)?;
        self.slots[b_existing]
            .player_ref_count
            .fetch_add(1, Ordering::AcqRel);
        self.presentation_queue.push_back(b_existing);

        if refresh_all_vbi {
            for i in 0..8 {
                if let Some(old_b) = self.vbi[i] {
                    self.slots[old_b]
                        .decoder_ref_count
                        .fetch_sub(1, Ordering::AcqRel);
                }
                self.vbi[i] = Some(b_existing);
                self.slots[b_existing]
                    .decoder_ref_count
                    .fetch_add(1, Ordering::AcqRel);
            }
        }
        Ok(b_existing)
    }

    /// Pops the next frame from the presentation queue and decrements `PlayerRefCount[b]`.
    pub fn release_presented_frame(&mut self) -> Option<usize> {
        let b = self.presentation_queue.pop_front()?;
        let prev = self.slots[b]
            .player_ref_count
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prev >= 1);
        Some(b)
    }

    /// Clears and zeroes the output buffer on a decoder error without polluting VBI state.
    pub fn rollback_on_error(&mut self, b_current: usize) {
        if let Some(slot) = self.slots.get_mut(b_current) {
            if let FrameStorage::HostAligned(ref mut arc_buf) = slot.storage {
                if let Some(buf) = Arc::get_mut(arc_buf) {
                    buf.zero_fill();
                }
            }
        }
    }

    /// Typestate discontinuous seek reset: zeroes `DecoderRefCount[0..M]` and clears VBI
    /// while leaving `PlayerRefCount[0..M]` and `presentation_queue` intact so the display
    /// renderer never experiences a use-after-free during scrubbing.
    pub fn discontinuous_seek_reset(&mut self) {
        self.vbi = [None; 8];
        for slot in &self.slots {
            slot.decoder_ref_count.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_limit_before_allocation() {
        assert!(CPUFrame::new(PixelFormat::Yuv444p, 8192, 8192).is_err());
        assert!(CPUFrame::new(PixelFormat::Yuv420p10, u32::MAX, u32::MAX).is_err());
        assert!(CPUFrame::new(PixelFormat::Yuv420p, 0, 1).is_err());
    }
    #[test]
    fn odd_dimensions_and_shared_planes() {
        let frame = CPUFrame::new(PixelFormat::Yuv420p10, 3, 3).unwrap();
        assert_eq!(frame.planes[0].data.len(), 18);
        assert_eq!(frame.planes[1].data.len(), 8);
        assert_eq!(frame.planes[1].width, 2);
        assert!(Arc::ptr_eq(
            &frame.planes[0].data,
            &frame.clone().planes[0].data
        ));
    }
    #[test]
    fn cache_aligned_superblock_buffer_and_dual_ref_vbi_state_machine() {
        let mut pool = DualRefBufferPool::new_host_pool(16, 320, 240, 1).unwrap();
        if let FrameStorage::HostAligned(ref buf) = pool.slots[0].storage {
            assert_eq!(buf.base_addr() % CACHE_LINE_ALIGN, 0);
            assert_eq!(buf.stride_bytes % CACHE_LINE_ALIGN, 0);
            assert!(buf.padded_width >= 384 + 2 * FILTER_HALO_MARGIN);
        }
        // Keyframe refreshes all 8 VBI slots (255) and is shown
        let b0 = pool.acquire_free_slot().unwrap();
        assert_eq!(b0, 0);
        pool.commit_decoded_frame(b0, 0xff, true, 1, 0).unwrap();
        assert_eq!(pool.slots[b0].dec_refs(), 8);
        assert_eq!(pool.slots[b0].player_refs(), 1);

        // Invisible ALTREF frame refreshes slot 1 only (show_frame = false)
        let b1 = pool.acquire_free_slot().unwrap();
        assert_eq!(b1, 1);
        pool.commit_decoded_frame(b1, 0b0000_0010, false, 2, 4)
            .unwrap();
        assert_eq!(pool.slots[b0].dec_refs(), 7);
        assert_eq!(pool.slots[b1].dec_refs(), 1);
        assert_eq!(pool.slots[b1].player_refs(), 0);

        // show_existing_frame outputs VBI[1] (b1) without allocating a new slot
        let shown = pool.show_existing_frame(1, false).unwrap();
        assert_eq!(shown, b1);
        assert_eq!(pool.slots[b1].player_refs(), 1);

        // Discontinuous seek zeroes DecoderRefCount while preserving PlayerRefCount!
        pool.discontinuous_seek_reset();
        assert_eq!(pool.slots[b0].dec_refs(), 0);
        assert_eq!(pool.slots[b1].dec_refs(), 0);
        assert_eq!(pool.slots[b0].player_refs(), 1);
        assert_eq!(pool.slots[b1].player_refs(), 1);
        assert_eq!(pool.release_presented_frame(), Some(b0));
        assert_eq!(pool.release_presented_frame(), Some(b1));
        assert!(pool.slots[b0].is_free());
        assert!(pool.slots[b1].is_free());
    }
}
