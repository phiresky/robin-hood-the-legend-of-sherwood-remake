/** Private ordinary-character cutout candidate. Visibility-outline and projectile policies are separate. */
export function decodeMask(mask) {
  const [width,height]=mask.box_size;
  if(!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<1||height<1||width*height>16*1024*1024)throw Error('Invalid mask dimensions');
  const data=mask.mask_data,pixels=new Uint8Array(width*height);let offset=0;
  for(let y=0;y<height;y++){
    const length=data[offset++];if(length===undefined)throw Error('Truncated mask row');
    const end=offset+length;let block=0;
    while(offset<end){
      const control=data[offset++],count=control&127;
      if(!count)throw Error('Empty mask run');
      const repeated=!!(control&128),value=repeated?data[offset++]:undefined;
      for(let i=0;i<count;i++){
        const byte=repeated?value:data[offset++];
        if(byte===undefined||offset>end||block>=Math.ceil(width/8))throw Error('Invalid mask run');
        for(let bit=0;bit<8&&block*8+bit<width;bit++)pixels[y*width+block*8+bit]=(byte>>(7-bit))&1;
        block++;
      }
    }
    if(offset!==end)throw Error('Invalid row length');
  }
  if(offset!==data.length)throw Error('Trailing mask data');return pixels;
}
export function appliesToCharacter(mask,active,layer,point) {
  if(typeof active!=='boolean')throw Error('Current mask activity is required');
  if(!active||mask.layer!==layer||!(mask.mask_type&1))return false;
  const line=mask.character_polyline;
  if(!Array.isArray(line)||line.length<2||line.some((p,i)=>p.length!==2||p.some(v=>!Number.isFinite(v))||(i&&p[0]<=line[i-1][0])))throw Error('Invalid character polyline');
  const [x,y]=point;if(!Number.isFinite(x)||!Number.isFinite(y))throw Error('Invalid actor map position');
  if(x<line[0][0]||x>line.at(-1)[0])return false;
  let i=1;while(line[i][0]<x)i++;
  const a=line[i-1],b=line[i];return a[1]+(x-a[0])*(b[1]-a[1])/(b[0]-a[0])>y;
}
/** Caller supplies current active masks/layer/map position independently from screen-frame origin. */
export function maskCharacterPixels(source,screenOrigin,actor,masks,{drawHidden=false}={}) {
  if(drawHidden)throw Error('Hidden-character outline composition is not implemented');
  if(source.data.length!==source.width*source.height*4)throw Error('Invalid source pixels');
  if(!screenOrigin.every(Number.isSafeInteger))throw Error('Native frame origin must use source-pixel integers');
  const out={...source,data:source.data.slice()},applied=[];
  for(const {id,mask,active}of masks){
    if(!appliesToCharacter(mask,active,actor.layer,actor.mapPosition))continue;
    const [mx,my]=mask.box_top_left,[mw,mh]=mask.box_size,[sx,sy]=screenOrigin;
    const x0=Math.max(mx,sx),y0=Math.max(my,sy),x1=Math.min(mx+mw,sx+source.width),y1=Math.min(my+mh,sy+source.height);
    if(x0>=x1||y0>=y1)continue;
    const bitmap=decodeMask(mask);let cleared=0;
    for(let y=y0;y<y1;y++)for(let x=x0;x<x1;x++)if(bitmap[(y-my)*mw+x-mx]){const i=((y-sy)*source.width+x-sx)*4+3;if(out.data[i])cleared++;out.data[i]=0;}
    applied.push({id,cleared});
  }
  return {pixels:out,applied};
}
