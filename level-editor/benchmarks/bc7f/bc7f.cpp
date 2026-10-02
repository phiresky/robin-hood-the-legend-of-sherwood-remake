// Runtime adapter; upstream BC7f source is fetched unchanged at a pinned revision.
#include "basisu_transcoder.h"
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <emscripten/emscripten.h>
static float linear[256];
static uint8_t srgb[4097];
extern "C" {
EMSCRIPTEN_KEEPALIVE void initialize() {
  basist::basisu_transcoder_init();
  for (int i=0;i<256;i++) { float s=i/255.f; linear[i]=s<=.04045f?s/12.92f:powf((s+.055f)/1.055f,2.4f); }
  for (int i=0;i<=4096;i++) { float l=i/4096.f; srgb[i]=uint8_t(std::lround(255*(l<=.0031308f?12.92f*l:1.055f*powf(l,1/2.4f)-.055f))); }
}
EMSCRIPTEN_KEEPALIVE void encode(const uint8_t* pixels, int w, int h, uint8_t* out, int level) {
  auto flags=level?basist::bc7f::cPackBC7FlagDefaultPartiallyAnalytical:basist::bc7f::cPackBC7FlagDefault;
  basist::color_rgba block[16];
  for(int y=0;y<h;y+=4)for(int x=0;x<w;x+=4){
    for(int j=0;j<4;j++)for(int i=0;i<4;i++)
      memcpy(&block[j*4+i],pixels+(std::min(y+j,h-1)*w+std::min(x+i,w-1))*4,4);
    basist::bc7f::fast_pack_bc7_auto_rgba(out,block,flags);out+=16;
  }
}
// Area box filter in linear light, with alpha independent of RGB (ownership data).
// Does not preserve alpha coverage; production only submits fully opaque textures.
EMSCRIPTEN_KEEPALIVE void mip(const uint8_t* src,int w,int h,uint8_t* dst) {
  int nw=std::max(1,w/2),nh=std::max(1,h/2);
  for(int y=0;y<nh;y++)for(int x=0;x<nw;x++){
    double x0=double(x)*w/nw,x1=double(x+1)*w/nw,y0=double(y)*h/nh,y1=double(y+1)*h/nh;
    double sum[4]={},area=(x1-x0)*(y1-y0);
    for(int sy=int(y0);sy<std::min(h,int(std::ceil(y1)));sy++)for(int sx=int(x0);sx<std::min(w,int(std::ceil(x1)));sx++){
      double weight=(std::min(x1,double(sx+1))-std::max(x0,double(sx)))*(std::min(y1,double(sy+1))-std::max(y0,double(sy)));
      auto p=src+(sy*w+sx)*4;
      for(int c=0;c<3;c++)sum[c]+=linear[p[c]]*weight;
      sum[3]+=p[3]*weight;
    }
    for(int c=0;c<3;c++)*dst++=srgb[std::clamp(int(std::lround(sum[c]/area*4096)),0,4096)];
    *dst++=uint8_t(std::lround(sum[3]/area));
  }
}
}
