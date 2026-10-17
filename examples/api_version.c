/* Tested by the C integration suite. No decoding is implied by handle creation. */
#include "codec.h"
#include <stdint.h>
int main(void) {
    DecoderHandle *handle = 0;
    if ((codec_get_abi_version() >> 16) != 1) return 1;
    if (codec_decoder_create(Av1, &handle) != Ok) return 2;
    codec_decoder_destroy(handle);
    /* Applications must check capabilities before offering AV1 playback. */
    return codec_get_capabilities() == UINT64_C(0) ? 0 : 3;
}
