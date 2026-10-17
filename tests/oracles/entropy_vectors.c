/* Offline oracle generator, never built or linked by Cargo. See README.md.
 * Uses independently compiled libaom encoder, decoder, and CDF adaptation. */
#include <assert.h>
#include <stdio.h>
#include "aom_dsp/entenc.h"
#include "aom_dsp/entdec.h"
#include "aom_dsp/prob.h"

int main(void) {
  for (int n = 2; n <= 16; ++n) {
    for (int adaptive = 0; adaptive <= 1; ++adaptive) {
      aom_cdf_prob initial[17], enc_cdf[17], dec_cdf[17];
      for (int j = 0; j < n; ++j) initial[j] = 32768 - ((j+1)*32768/n);
      initial[n] = 0;
      for (int j = 0; j <= n; ++j) enc_cdf[j] = dec_cdf[j] = initial[j];
      od_ec_enc enc; od_ec_enc_init(&enc, 1024);
      int symbols[96];
      unsigned seed = 0x12345678;
      for (int i = 0; i < 96; ++i) {
        seed = seed*1664525u + 1013904223u;
        symbols[i] = (seed >> 16) % n;
        od_ec_encode_cdf_q15(&enc, symbols[i], enc_cdf, n);
        if (adaptive) update_cdf(enc_cdf, symbols[i], n);
      }
      uint32_t size; unsigned char *buf = od_ec_enc_done(&enc,&size);
      assert(buf != NULL);
      od_ec_dec dec; od_ec_dec_init(&dec,buf,size);
      for (int i = 0; i < 96; ++i) {
        int got = od_ec_decode_cdf_q15(&dec,dec_cdf,n);
        assert(got == symbols[i]);
        if (adaptive) update_cdf(dec_cdf,got,n);
      }
      for (int j = 0; j <= n; ++j) assert(enc_cdf[j] == dec_cdf[j]);
      printf("%d %d ",n,adaptive);
      for (unsigned j=0;j<size;++j) printf("%02x",buf[j]);
      printf(" "); for(int j=0;j<96;++j) printf("%s%d",j?",":"",symbols[j]);
      printf(" "); for(int j=0;j<n;++j) printf("%s%d",j?",":"",32768-dec_cdf[j]);
      printf(",%d\n",dec_cdf[n]);
      od_ec_enc_clear(&enc);
    }
  }
}
