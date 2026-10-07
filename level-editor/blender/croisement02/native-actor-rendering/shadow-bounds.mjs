/** Bound only explicitly decoded shadow alpha, in the shared source-frame coordinates. */
export function shadowCoverageBounds(pixels,bounds) {
  const {left,top,width,height}=bounds??{};
  if(![left,top,width,height].every(Number.isSafeInteger)||width<=0||height<=0||
      !(pixels instanceof Uint8Array || pixels instanceof Uint8ClampedArray)||pixels.length!==width*height*4)
    throw Error('Invalid decoded shadow frame');
  let x0=width,y0=height,x1=-1,y1=-1;
  for(let y=0;y<height;y++)for(let x=0;x<width;x++)if(pixels[(y*width+x)*4+3]){
    x0=Math.min(x0,x);y0=Math.min(y0,y);x1=Math.max(x1,x);y1=Math.max(y1,y);
  }
  return x1<0?null:{left:left+x0,top:top-y0,width:x1-x0+1,height:y1-y0+1};
}
