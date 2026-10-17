#ifndef CODEC_H
#define CODEC_H

#include <stdint.h>
typedef struct DecoderHandle DecoderHandle;
typedef struct CodecFrame CodecFrame;
typedef struct CodecPipeline CodecPipeline;
typedef struct CodecVulkanConfig { uint64_t device; uint64_t decode_queue; uint32_t queue_family_index; } CodecVulkanConfig;
typedef struct CodecVulkanFrameDescriptor { uint64_t image_view; uint64_t timeline_semaphore; uint64_t timeline_value; uint8_t dpb_slot; } CodecVulkanFrameDescriptor;
typedef struct CodecSyncAdjustment { double phase_error_sec; double control_signal; double audio_resample_ratio; } CodecSyncAdjustment;
typedef enum FfiCodecId { Av1 = 0, H264 = 1 } FfiCodecId;
#define CODEC_ERR_INTERNAL_PANIC InternalPanic

typedef enum FfiResult {
  Ok = 0,
  InvalidData = -1,
  Unsupported = -2,
  OutOfMemory = -3,
  InternalError = -4,
  InternalPanic = -5,
} FfiResult;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

const char *codec_get_version(void);

/**
 * C ABI major in the upper 16 bits, additive minor in the lower 16 bits.
 * Major 1 freezes the documented existing symbols and layouts.
 */
uint32_t codec_get_abi_version(void);

/**
 * Runtime backend capabilities: bit 0 = complete AV1 pictures, bit 1 = Vulkan
 * Video decode, bit 2 = film-grain synthesis. Zero means none are available.
 * Descriptor/header helpers do not imply any of these capabilities.
 */
uint64_t codec_get_capabilities(void);

/**
 * # Safety
 * out_handle must be non-NULL and writable; failures clear *out_handle.
 */
enum FfiResult codec_decoder_create(uint32_t codec, struct DecoderHandle **out_handle);

/**
 * # Safety
 * Pass NULL or one live handle returned by create, exactly once.
 */
void codec_decoder_destroy(struct DecoderHandle *handle);

/**
 * # Safety
 * handle and config must be valid non-NULL pointers.
 */
enum FfiResult codec_decoder_set_vulkan_context(struct DecoderHandle *handle, const struct CodecVulkanConfig *config);

/**
 * # Safety
 * frame and out_desc must be valid non-NULL pointers.
 */
enum FfiResult codec_frame_export_vulkan_image(const struct CodecFrame *frame, struct CodecVulkanFrameDescriptor *out_desc);

/**
 * # Safety
 * dsl_source must be a valid NUL-terminated UTF-8 string and out_pipeline must be non-NULL.
 */
enum FfiResult codec_pipeline_compile_dsl(const char *dsl_source, struct CodecPipeline **out_pipeline);

/**
 * # Safety
 * pipeline and out_adj must be valid non-NULL pointers.
 */
enum FfiResult codec_pipeline_step_sync(struct CodecPipeline *pipeline, double audio_pts, double vsync_pts, struct CodecSyncAdjustment *out_adj);

/**
 * # Safety
 * Pass NULL or one live pipeline handle returned by codec_pipeline_compile_dsl, exactly once.
 */
void codec_pipeline_destroy(struct CodecPipeline *pipeline);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* CODEC_H */
