/* Independent 4x4 coefficient-context vectors using unmodified pinned libaom.
 * Contexts and scan order come from libaom, not this repository's decoder.
 * The Rust comparison transposes libaom's column-major coefficient storage. */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "aom_dsp/entenc.h"
#include "aom_dsp/prob.h"
#include "av1/common/scan.h"
#include "av1/common/token_cdfs.h"
#include "av1/common/txb_common.h"

static void symbol(od_ec_enc *enc, int value, aom_cdf_prob *cdf, int n) {
  od_ec_encode_cdf_q15(enc, value, cdf, n);
  update_cdf(cdf, value, n);
}
static void bit(od_ec_enc *enc, int value) {
  const aom_cdf_prob cdf[3] = { 16384, 0, 0 };
  od_ec_encode_cdf_q15(enc, value, cdf, 2);
}
static void golomb(od_ec_enc *enc, unsigned value) {
  unsigned word = value + 1;
  int bits = 0;
  for (unsigned v = word; v; v >>= 1) ++bits;
  for (int i = 1; i < bits; ++i) bit(enc, 0);
  for (int i = bits - 1; i >= 0; --i) bit(enc, (word >> i) & 1);
}
int main(void) {
  const TX_TYPE kinds[] = { DCT_DCT, ADST_DCT, DCT_ADST, ADST_ADST, IDTX, V_DCT, H_DCT };
  for (int q = 0; q < 4; ++q) for (int k = 0; k < 7; ++k) for (int test = 0; test < 3; ++test) {
    aom_cdf_prob skip[3], eob[6], extra[3], base_eob[4][4], base[42][5], br[21][5], dc[3];
    TX_TYPE kind = kinds[k];
    TX_CLASS tx_class = tx_type_to_class[kind];
    const int16_t *scan = av1_scan_orders[TX_4X4][kind].scan;
    memcpy(skip, av1_default_txb_skip_cdfs[q][0][0], sizeof(skip));
    memcpy(eob, av1_default_eob_multi16_cdfs[q][0][tx_class != TX_CLASS_2D], sizeof(eob));
    memcpy(extra, av1_default_eob_extra_cdfs[q][0][0][2], sizeof(extra));
    memcpy(base_eob, av1_default_coeff_base_eob_multi_cdfs[q][0][0], sizeof(base_eob));
    memcpy(base, av1_default_coeff_base_multi_cdfs[q][0][0], sizeof(base));
    memcpy(br, av1_default_coeff_lps_multi_cdfs[q][0][0], sizeof(br));
    memcpy(dc, av1_default_dc_sign_cdfs[q][0][0], sizeof(dc));
    uint8_t levels[64] = { 0 };
    int coefficients[16];
    for (int i = 0; i < 16; ++i) {
      int magnitude = test == 0 ? (i % 3 ? 1 : 0) : test == 1 ? 3 : 16;
      if (i == scan[15] && magnitude == 0) magnitude = 1;
      coefficients[i] = i % 2 ? -magnitude : magnitude;
      levels[get_padded_idx(i, 2)] = magnitude > 15 ? 15 : magnitude;
    }
    od_ec_enc enc;
    od_ec_enc_init(&enc, 256);
    symbol(&enc, 0, skip, 2);
    symbol(&enc, 4, eob, 5); /* EOB point 5, EOB 16 = 9 + 7. */
    symbol(&enc, 1, extra, 2);
    bit(&enc, 1); bit(&enc, 1);
    for (int c = 15; c >= 0; --c) {
      int pos = scan[c], magnitude = abs(coefficients[pos]);
      int ctx = get_lower_levels_ctx_general(c == 15, c, 2, 4, levels, pos, TX_4X4, tx_class);
      if (c == 15) symbol(&enc, magnitude > 3 ? 2 : magnitude - 1, base_eob[ctx], 3);
      else symbol(&enc, magnitude > 3 ? 3 : magnitude, base[ctx], 4);
      if (magnitude >= 3) {
        int remaining = magnitude - 3;
        int br_ctx = c == 15 ? get_br_ctx_eob(pos, 2, tx_class) : get_br_ctx(levels, pos, 2, tx_class);
        for (int j = 0; j < 4; ++j) {
          int value = remaining > 3 ? 3 : remaining;
          symbol(&enc, value, br[br_ctx], 4);
          remaining -= value;
          if (value < 3) break;
        }
      }
    }
    for (int c = 0; c < 16; ++c) {
      int value = coefficients[scan[c]], magnitude = abs(value);
      if (!magnitude) continue;
      if (c == 0) symbol(&enc, value < 0, dc, 2);
      else bit(&enc, value < 0);
      if (magnitude >= 15) golomb(&enc, magnitude - 15);
    }
    for (int i = 7; i >= 0; --i) bit(&enc, (0xa5 >> i) & 1);
    uint32_t size;
    unsigned char *bytes = od_ec_enc_done(&enc, &size);
    assert(bytes != NULL);
    printf("%d %d %d ", q, k, test);
    for (unsigned i = 0; i < size; ++i) printf("%02x", bytes[i]);
    for (int row = 0; row < 4; ++row) for (int col = 0; col < 4; ++col)
      printf(" %d", coefficients[col * 4 + row]);
    printf("\n");
    od_ec_enc_clear(&enc);
  }
}
