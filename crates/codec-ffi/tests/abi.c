#include "codec.h"
#include <assert.h>
#include <stddef.h>
#include <string.h>

int main(void) {
    DecoderHandle *handle = NULL;
    CodecPipeline *pipeline = NULL;
    CodecVulkanConfig vk_cfg = { 0x1000u, 0x2000u, 0u };
    CodecSyncAdjustment sync_adj = { 0.0, 0.0, 0.0 };

    assert(codec_get_abi_version() == UINT32_C(0x00010000));
    assert(codec_get_capabilities() == UINT64_C(0));
    _Static_assert(sizeof(FfiResult) == sizeof(int32_t), "ABI requires 32-bit C enums");
    _Static_assert(sizeof(FfiCodecId) == sizeof(int32_t), "ABI requires 32-bit C enums");
    _Static_assert(offsetof(CodecVulkanConfig, decode_queue) == 8, "Vulkan config ABI");
    _Static_assert(offsetof(CodecVulkanConfig, queue_family_index) == 16, "Vulkan config ABI");
    _Static_assert(offsetof(CodecVulkanFrameDescriptor, timeline_value) == 16, "descriptor ABI");
    _Static_assert(offsetof(CodecVulkanFrameDescriptor, dpb_slot) == 24, "descriptor ABI");
    _Static_assert(sizeof(CodecSyncAdjustment) == 3 * sizeof(double), "sync ABI");
    assert(strcmp(codec_get_version(), "0.1.0") == 0);
    assert(codec_decoder_create(Av1, &handle) == Ok);
    assert(handle != NULL);
    assert(codec_decoder_set_vulkan_context(handle, &vk_cfg) == Ok);
    assert(codec_decoder_set_vulkan_context(handle, NULL) == InvalidData);
    codec_decoder_destroy(handle);
    handle = NULL;

    assert(codec_decoder_create(H264, &handle) == Unsupported);
    assert(handle == NULL);
    assert(codec_decoder_create(UINT32_MAX, &handle) == InvalidData);
    assert(handle == NULL);
    assert(codec_decoder_create(Av1, NULL) == InvalidData);
    codec_decoder_destroy(NULL);

    assert(codec_pipeline_compile_dsl("film_grain(gpu: true); tonemap(curve: mobius); interpolate(model: rife_v4_tensorrt)", &pipeline) == Ok);
    assert(pipeline != NULL);
    assert(codec_pipeline_step_sync(pipeline, 1.0, 1.0, &sync_adj) == Ok);
    assert(sync_adj.audio_resample_ratio >= 0.995 && sync_adj.audio_resample_ratio <= 1.005);
    codec_pipeline_destroy(pipeline);
    codec_pipeline_destroy(NULL);

    assert(codec_pipeline_compile_dsl("", &pipeline) == InvalidData);
    assert(pipeline == NULL);
    assert(CODEC_ERR_INTERNAL_PANIC == InternalPanic);
    return 0;
}
