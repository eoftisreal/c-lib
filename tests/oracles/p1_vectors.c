/* Independent libaom oracle; no reference library is linked by Cargo. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include "config/aom_config.h"
#include "config/av1_rtcd.h"
#include "av1/common/resize.h"
#include "av1/common/cdef.h"
#include "aom_ports/mem.h"
#include "av1/common/restoration.h"
#include "av1/common/convolve.h"
static uint32_t seed=0xa93271d1u;
static uint32_t rnd(void){seed=seed*1664525u+1013904223u;return seed;}
static void print_values(const uint16_t *v,int n){for(int i=0;i<n;++i){printf("%s%u",i?",":"",v[i]);}}
int main(void){
  const int widths[]={1,2,3,4,7,8,9,16,17,31,32,63,64,127,128,255,257};
  for(int bd=8;bd<=12;bd+=2)for(unsigned wi=0;wi<sizeof(widths)/sizeof(widths[0]);++wi)for(int denom=9;denom<=16;++denom)for(int pattern=0;pattern<4;++pattern){
    const int up=widths[wi],down=(up*8+denom/2)/denom;
    SequenceHeader seq={0};seq.use_highbitdepth=1;seq.bit_depth=(aom_bit_depth_t)bd;seq.mib_size_log2=4;
    AV1_COMMON cm={0};cm.seq_params=&seq;cm.width=down;cm.superres_upscaled_width=up;cm.superres_scale_denominator=denom;
    cm.mi_params.mi_cols=2*((down+7)/8);cm.tiles.cols=1;cm.tiles.col_start_sb[1]=(cm.mi_params.mi_cols+15)/16;
    uint16_t storage[400]={0},out[300]={0};uint16_t *in=storage+8;const int mask=(1<<bd)-1;
    for(int x=0;x<down;++x){in[x]=pattern==0?mask/2:pattern==1?(x&1?mask:0):pattern==2?(x==down/2?mask:0):((int)rnd()&mask);}
    for(int x=down;x<cm.mi_params.mi_cols*4;++x)in[x]=in[down-1];
    if(up==down){for(int x=0;x<up;++x)out[x]=in[x];}
    else av1_upscale_normative_rows(&cm,CONVERT_TO_BYTEPTR(in),384,CONVERT_TO_BYTEPTR(out),300,0,1);
    printf("S %d %d %d ",bd,down,up);print_values(in,down);printf(" ");print_values(out,up);printf("\n");
  }
  for(int bd=8;bd<=12;bd+=2)for(int dir=0;dir<8;++dir)for(int pri=0;pri<=15;++pri)for(int secidx=0;secidx<4;++secidx)for(int edge=0;edge<2;++edge){
    const int sec=(int[]){0,1,2,4}[secidx],shift=bd-8,damp=2+(pri%5),w=16,h=16,x0=edge?0:4,y0=edge?0:4;
    const uint32_t initial_seed=seed;
    uint16_t image[256],in[CDEF_BSTRIDE*24],out[64];
    for(int i=0;i<256;++i){int v=128+(int)(rnd()%25)-12;image[i]=(uint16_t)(v<<shift);}
    for(unsigned i=0;i<sizeof(in)/sizeof(in[0]);++i)in[i]=CDEF_VERY_LARGE;
    for(int y=0;y<h;++y)for(int x=0;x<w;++x)in[(y+2)*CDEF_BSTRIDE+x+8]=image[y*w+x];
    const int primary=(pri<<shift)-((bd>8&&pri>0)?pri%3:0),secondary=sec<<shift;
    cdef_filter_16_0_c(out,8,in+(y0+2)*CDEF_BSTRIDE+x0+8,primary,secondary,dir,damp+shift,damp+shift,shift,8,8);
    printf("C %d %d %d %d %d %d %d %u ",bd,dir,primary,secondary,damp,x0,y0,initial_seed);print_values(out,64);int32_t variance=0;int direction=cdef_find_dir_c(image+y0*w+x0,w,&variance,shift);printf(" %d %d\n",direction,variance);
  }

  av1_loop_restoration_precal();
  for(int kind=0;kind<2;++kind)for(int bd=8;bd<=12;bd+=2)for(int mode=0;mode<3;++mode)for(int test=0;test<(kind?64:32);++test){
    const int x0=mode==2?0:4,y0=mode==2?0:4,start=mode==1?4:0,end=mode==0?16:mode==1?12:8,mask=(1<<bd)-1;
    const uint32_t initial_seed=seed;uint16_t pre[256],post[256],in[24*32]={0},out[64];
    for(int i=0;i<256;++i){pre[i]=rnd()&mask;post[i]=rnd()&mask;}
    uint16_t *base=in+4*32+8;
    for(int y=-4;y<12;++y)for(int x=-4;x<12;++x){
      int xx=x0+x,yy=y0+y;if(xx<0)xx=0;if(xx>15)xx=15;if(yy<0)yy=0;if(yy>15)yy=15;
      uint16_t value;if(yy<start){if(yy<start-2)yy=start-2;value=pre[yy*16+xx];}
      else if(yy>=end){if(yy>end+1)yy=end+1;value=pre[yy*16+xx];}
      else {value=post[yy*16+xx];}
      base[y*32+x]=value;
    }
    if(!kind){
      const int lo[]={-5,-23,-17},range[]={16,32,64};int hv[3],vv[3];
      DECLARE_ALIGNED(256,int16_t,hfilter[16][8])={{0}};DECLARE_ALIGNED(256,int16_t,vfilter[16][8])={{0}};
      for(int i=0;i<3;++i){hv[i]=test==0?0:test==1?lo[i]:test==2?lo[i]+range[i]-1:lo[i]+((test*7+i*13)%range[i]);vv[i]=test==0?0:test==1?lo[i]+range[i]-1:test==2?lo[i]:lo[i]+((test*11+i*17)%range[i]);hfilter[0][i]=hfilter[0][6-i]=hv[i];vfilter[0][i]=vfilter[0][6-i]=vv[i];hfilter[0][3]-=2*hv[i];vfilter[0][3]-=2*vv[i];}
      WienerConvolveParams params=get_conv_params_wiener(bd);
      av1_highbd_wiener_convolve_add_src_c(CONVERT_TO_BYTEPTR(base),32,CONVERT_TO_BYTEPTR(out),8,hfilter[0],16,vfilter[0],16,8,8,&params,bd);
      printf("W %d %d %u %d,%d,%d %d,%d,%d ",bd,mode,initial_seed,hv[0],hv[1],hv[2],vv[0],vv[1],vv[2]);print_values(out,64);printf("\n");
    }else{
      const int set=test/4,which=test%4;int xqd[]={ (int[]){-96,0,31,-33}[which],(int[]){-32,0,95,44}[which]};
      if(av1_sgr_params[set].r[0]==0){xqd[0]=0;}
      if(av1_sgr_params[set].r[1]==0){xqd[1]=95;}
      static int32_t tmp[RESTORATION_TMPBUF_SIZE/sizeof(int32_t)];
      assert(av1_apply_selfguided_restoration_c(CONVERT_TO_BYTEPTR(base),8,8,32,set,xqd,CONVERT_TO_BYTEPTR(out),8,tmp,bd,1)==0);
      printf("G %d %d %u %d %d %d ",bd,mode,initial_seed,set,xqd[0],xqd[1]);print_values(out,64);printf("\n");
    }
  }
  return 0;
}
