/* Offline differential fixture generator. Link against unmodified libaom.
 * No reference implementation is linked by Cargo. See README.md. */
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include "config/aom_config.h"
#include "config/aom_dsp_rtcd.h"
#include "config/av1_rtcd.h"
#include "av1/common/av1_inv_txfm1d.h"
#include "av1/common/av1_inv_txfm2d.h"
#include "av1/common/quant_common.h"
#include "av1/common/scan.h"
#include "aom_ports/mem.h"
#include "aom_dsp/entenc.h"
#include "aom_dsp/entdec.h"
#include "aom_dsp/prob.h"
#include "av1/common/av1_common_int.h"
void av1_init_mode_probs(FRAME_CONTEXT *fc);

static uint32_t seed = 0x37b293a1;
static uint32_t random_word(void) { seed = seed * 1664525u + 1013904223u; return seed; }
static void values(const int32_t *v, int n) { for(int i=0;i<n;++i) { printf("%s%d",i?",":"",v[i]); } }

int main(void) {
  typedef void (*kernel)(const int32_t *, int32_t *, int8_t, const int8_t *);
  const kernel kernels[3][5] = {
    {av1_idct4,av1_idct8,av1_idct16,av1_idct32,av1_idct64},
    {av1_iadst4,av1_iadst8,av1_iadst16,NULL,NULL},
    {av1_iidentity4_c,av1_iidentity8_c,av1_iidentity16_c,av1_iidentity32_c,NULL}
  };
  for(int kind=0;kind<3;++kind) for(int size=0;size<5;++size) {
    if (!kernels[kind][size]) continue;
    const int n = 4<<size;
    for(int r=16;r<=20;++r) for(int k=0;k<n+32;++k) {
      int32_t in[64]={0},out[64]={0}; int8_t ranges[12]; memset(ranges,r,sizeof(ranges));
      if(k< n) in[k]=(k&1)?-256:256;
      else for(int j=0;j<n;++j) in[j]=(int)(random_word()>>16)%513-256;
      kernels[kind][size](in,out,12,ranges);
      printf("T %d %d %d ",kind,n,r); values(in,n); printf(" "); values(out,n); printf("\n");
    }
  }
  for(int bd=8;bd<=12;bd+=2) for(int q=0;q<256;++q)
    printf("Q %d %d %d %d\n",bd,q,av1_dc_quant_QTX(q,0,(aom_bit_depth_t)bd),av1_ac_quant_QTX(q,0,(aom_bit_depth_t)bd));

#define SMOOTH_CASE(w,h) \
  for(int bd=8;bd<=12;bd+=2) for(int kind=0;kind<3;++kind) { \
    uint16_t above[w],left[h],out[w*h]; \
    for(int i=0;i<w;++i) above[i]=random_word()&((1<<bd)-1); \
    for(int i=0;i<h;++i) left[i]=random_word()&((1<<bd)-1); \
    if(kind==0) aom_highbd_smooth_predictor_##w##x##h##_c(out,w,above,left,bd); \
    if(kind==1) aom_highbd_smooth_v_predictor_##w##x##h##_c(out,w,above,left,bd); \
    if(kind==2) aom_highbd_smooth_h_predictor_##w##x##h##_c(out,w,above,left,bd); \
    printf("S %d %d %d %d ",kind,w,h,bd); \
    for(int i=0;i<w;++i) { printf("%s%u",i?",":"",above[i]); } printf(" "); \
    for(int i=0;i<h;++i) { printf("%s%u",i?",":"",left[i]); } printf(" "); \
    for(int i=0;i<w*h;++i) { printf("%s%u",i?",":"",out[i]); } printf("\n"); \
  }
  SMOOTH_CASE(4,4) SMOOTH_CASE(4,8) SMOOTH_CASE(8,4) SMOOTH_CASE(8,8)
  SMOOTH_CASE(8,16) SMOOTH_CASE(16,8) SMOOTH_CASE(16,16)
  SMOOTH_CASE(16,32) SMOOTH_CASE(32,16) SMOOTH_CASE(32,32)
  SMOOTH_CASE(32,64) SMOOTH_CASE(64,32) SMOOTH_CASE(64,64)
  SMOOTH_CASE(4,16) SMOOTH_CASE(16,4) SMOOTH_CASE(8,32) SMOOTH_CASE(32,8)
  SMOOTH_CASE(16,64) SMOOTH_CASE(64,16)

  /* libaom's WHT coefficient/spatial layout is transposed relative to the
   * specification's row-first process. Transpose coefficients for this fixture. */
  for(int bd=8;bd<=12;bd+=2) for(int k=0;k<64;++k) {
    int32_t in[16]={0}, transposed[16], expected[16]; uint16_t out[16];
    for(int j=0;j<16;++j) { in[j] = k<16 ? (j==k?64:0) : ((int)(random_word()>>16)%65-32)*4; out[j]=1<<(bd-1); }
    for(int y=0;y<4;++y) for(int x=0;x<4;++x) transposed[x*4+y]=in[y*4+x];
    av1_highbd_iwht4x4_16_add_c(transposed,CONVERT_TO_BYTEPTR(out),4,bd);
    for(int y=0;y<4;++y) for(int x=0;x<4;++x) expected[y*4+x]=(int32_t)out[y*4+x]-(1<<(bd-1));
    printf("W %d ",bd); values(in,16); printf(" "); values(expected,16); printf("\n");
  }

  typedef void (*kernel2d)(const int32_t *, uint16_t *, int, TX_TYPE, int);
  const kernel2d kernels2d[] = {
    av1_inv_txfm2d_add_4x4_c,av1_inv_txfm2d_add_8x8_c,av1_inv_txfm2d_add_16x16_c,
    av1_inv_txfm2d_add_32x32_c,av1_inv_txfm2d_add_64x64_c,
    av1_inv_txfm2d_add_4x8_c,av1_inv_txfm2d_add_8x4_c,av1_inv_txfm2d_add_8x16_c,av1_inv_txfm2d_add_16x8_c,
    av1_inv_txfm2d_add_16x32_c,av1_inv_txfm2d_add_32x16_c,av1_inv_txfm2d_add_32x64_c,av1_inv_txfm2d_add_64x32_c,
    av1_inv_txfm2d_add_4x16_c,av1_inv_txfm2d_add_16x4_c,av1_inv_txfm2d_add_8x32_c,av1_inv_txfm2d_add_32x8_c,
    av1_inv_txfm2d_add_16x64_c,av1_inv_txfm2d_add_64x16_c
  };
  const int widths[]={4,8,16,32,64,4,8,8,16,16,32,32,64,4,16,8,32,16,64};
  const int heights[]={4,8,16,32,64,8,4,16,8,32,16,64,32,16,4,32,8,64,16};
  for(int size=0;size<19;++size) for(int kind=0;kind<16;++kind) {
    const int w=widths[size],h=heights[size],max=w>h?w:h,cw=w<32?w:32,ch=h<32?h:32;
    if((max==64 && kind!=0)||(max==32 && kind!=0 && kind!=9)) continue;
    printf("C %d %d ",size,kind);
    for(int i=0;i<cw*ch;++i) {int pos=av1_scan_orders[size][kind].scan[i];printf("%s%d",i?",":"",(pos%ch)*cw+pos/ch);}
    printf("\n");
    for(int bd=8;bd<=12;bd+=2) for(int pattern=0;pattern<5;++pattern) {
      int32_t in[1024]={0}; uint16_t out[4096];
      const uint32_t initial_seed=seed;
      for(int y=0;y<ch;++y) for(int x=0;x<cw;++x) {
        int i=y*cw+x,v=0;
        if(pattern==1) v=i==0?64:0;
        if(pattern==2) v=i==cw+1?-32:0;
        if(pattern==3) v=(int)(random_word()>>16)%65-32;
        if(pattern==4) v=(i&1)?-64:64;
        in[x*ch+y]=v*(1<<(bd-8));
      }
      for(int i=0;i<w*h;++i) out[i]=1<<(bd-1);
      kernels2d[size](in,out,w,(TX_TYPE)kind,bd);
      printf("D %d %d %d %d %" PRIu32 " ",size,kind,bd,pattern,initial_seed);
      for(int i=0;i<w*h;++i) { printf("%s%u",i?",":"",out[i]); } printf("\n");
    }
  }

  for(int log=3;log<=7;++log) for(int ctx=0;ctx<4;++ctx)
    for(int edge=0;edge<4;++edge) for(int adaptive=0;adaptive<2;++adaptive) {
      if(log==3 && edge!=0 && edge!=3) continue;
      FRAME_CONTEXT fc={0}; av1_init_mode_probs(&fc);
      aom_cdf_prob *cdf=fc.partition_cdf[4*(log-3)+ctx];
      const BLOCK_SIZE bs = log==3?BLOCK_8X8:log==4?BLOCK_16X16:log==5?BLOCK_32X32:log==6?BLOCK_64X64:BLOCK_128X128;
      const int n = log==3?4:log==7?8:10;
      od_ec_enc enc; od_ec_enc_init(&enc,1024); int symbols[96];
      for(int i=0;i<96;++i) {
        const int sym=(random_word()>>16)%(edge==0?n:2);
        if(edge==0) {
          od_ec_encode_cdf_q15(&enc,sym,cdf,n);
          if(adaptive) update_cdf(cdf,sym,n);
          symbols[i]=sym;
        } else if(edge==3) { symbols[i]=PARTITION_SPLIT; }
        else {
          aom_cdf_prob binary[2];
          if(edge==1) partition_gather_vert_alike(binary,cdf,bs);
          else partition_gather_horz_alike(binary,cdf,bs);
          od_ec_encode_cdf_q15(&enc,sym,binary,2);
          symbols[i]=sym?PARTITION_SPLIT:(edge==1?PARTITION_HORZ:PARTITION_VERT);
        }
      }
      const aom_cdf_prob binary[]={16384,0};
      for(int i=7;i>=0;--i) od_ec_encode_cdf_q15(&enc,(0xa5>>i)&1,binary,2);
      uint32_t size; const unsigned char *buf=od_ec_enc_done(&enc,&size); assert(buf!=NULL);
      printf("P %d %d %d %d ",1<<log,ctx,edge,adaptive);
      for(uint32_t j=0;j<size;++j) { printf("%02x",buf[j]); } printf(" ");
      values(symbols,96); printf("\n"); od_ec_enc_clear(&enc);
    }
  return 0;
}
