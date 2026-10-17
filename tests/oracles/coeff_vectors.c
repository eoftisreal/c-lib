/* Offline regression vectors using the pinned libaom range encoder and CDFs.
 * Symbols follow AV1 read_coeffs for a luma 4x4 block, empty neighbor contexts.
 * The trailing 0xa5 literal detects incorrect range-symbol consumption. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "aom_dsp/entenc.h"
#include "aom_dsp/prob.h"
#include "av1/common/token_cdfs.h"

static void symbol(od_ec_enc *enc, int value, aom_cdf_prob *cdf, int n) {
  od_ec_encode_cdf_q15(enc, value, cdf, n);
  update_cdf(cdf, value, n);
}
static void bit(od_ec_enc *enc, int value) {
  const aom_cdf_prob cdf[3] = {16384, 0, 0};
  od_ec_encode_cdf_q15(enc, value, cdf, 2);
}
int main(void) {
  for (int test = 0; test < 4; ++test) {
    aom_cdf_prob skip[3], eob[6], base_eob[4][4], base[42][5], br[21][5], dc[3];
    memcpy(skip, av1_default_txb_skip_cdfs[0][0][0], sizeof(skip));
    memcpy(eob, av1_default_eob_multi16_cdfs[0][0][0], sizeof(eob));
    memcpy(base_eob, av1_default_coeff_base_eob_multi_cdfs[0][0][0], sizeof(base_eob));
    memcpy(base, av1_default_coeff_base_multi_cdfs[0][0][0], sizeof(base));
    memcpy(br, av1_default_coeff_lps_multi_cdfs[0][0][0], sizeof(br));
    memcpy(dc, av1_default_dc_sign_cdfs[0][0][0], sizeof(dc));
    od_ec_enc enc;
    od_ec_enc_init(&enc, 128);
    symbol(&enc, 0, skip, 2);
    symbol(&enc, test == 1 ? 1 : 0, eob, 5);
    if (test == 1) {
      symbol(&enc, 0, base_eob[1], 3); /* AC level 1 */
      symbol(&enc, 3, base[0], 4);    /* DC level 3 */
      symbol(&enc, 0, br[1], 4);      /* required even for level 3 */
    } else {
      symbol(&enc, 2, base_eob[0], 3);
      if (test == 0) symbol(&enc, 0, br[0], 4);
      else for (int i = 0; i < 4; ++i) symbol(&enc, 3, br[0], 4);
    }
    symbol(&enc, test == 2, dc, 2);
    if (test == 1) bit(&enc, 1); /* negative AC */
    if (test == 2) bit(&enc, 1); /* Golomb 0 */
    if (test == 3) { bit(&enc, 0); bit(&enc, 1); bit(&enc, 0); } /* Golomb 1 */
    for (int i = 7; i >= 0; --i) bit(&enc, (0xa5 >> i) & 1);
    uint32_t size;
    unsigned char *buf = od_ec_enc_done(&enc, &size);
    assert(buf != NULL);
    printf("%d ", test);
    for (unsigned i = 0; i < size; ++i) printf("%02x", buf[i]);
    printf(" %d %d\n", test == 2 ? -15 : test == 3 ? 16 : 3, test == 1 ? -1 : 0);
    od_ec_enc_clear(&enc);
  }
}
