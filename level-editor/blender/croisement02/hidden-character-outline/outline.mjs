import { decodeMask, appliesToCharacter } from '../native-character-masks/character-masks.mjs';

export function packColor(r, g, b, depth = 16) {
  if (![15,16].includes(depth)) throw Error('Unsupported packed depth');
  return depth === 16 ? ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
    : ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
}

/** Decode unfiltered legacy RGBA into a packed, pre-mask sprite surface. */
export function decodeLegacy(source, {depth = 16, shadowKey} = {}) {
  if (![15,16].includes(depth) || !Number.isInteger(shadowKey) || shadowKey < 0 || shadowKey >= 2 ** depth)
    throw Error('Explicit packed depth and decompressed shadow key required');
  if (!Number.isSafeInteger(source.width) || !Number.isSafeInteger(source.height) ||
      source.width < 1 || source.height < 1 || source.width * source.height > 16777216 ||
      source.data.length !== source.width * source.height * 4) throw Error('Invalid source image');
  const transparent = depth === 16 ? 0x07c0 : 0x03e0;
  if (shadowKey === transparent) throw Error('Shadow key collides with transparency');
  const data = new Uint16Array(source.width * source.height);
  for (let i = 0; i < data.length; i++) {
    const [r,g,b,a] = source.data.slice(i*4,i*4+4);
    if (a !== 0 && a !== 255) throw Error('Filtered alpha is not a native packed sprite');
    const raw = packColor(r,g,b,16);
    data[i] = a === 0 || raw === 0x07c0 ? transparent
      : raw === 0x001f ? shadowKey : packColor(r,g,b,depth);
  }
  return {width:source.width,height:source.height,data,depth,transparent,shadowKey};
}

/** Apply ordered masks to an isolated packed sprite; not a silhouette dilation. */
export function outlineCharacter(source, screenOrigin, actor, masks, {outlineColor, drawHidden = true} = {}) {
  if (!(source.data instanceof Uint16Array) || source.data.length !== source.width * source.height ||
      ![15,16].includes(source.depth) || !Number.isInteger(outlineColor) || outlineColor < 0 ||
      outlineColor >= 2 ** source.depth || outlineColor === source.transparent || outlineColor === source.shadowKey)
    throw Error('Invalid packed surface/outline color');
  if (typeof drawHidden !== 'boolean' || !Array.isArray(screenOrigin) || screenOrigin.length !== 2 ||
      !screenOrigin.every(Number.isSafeInteger)) throw Error('Integer native frame origin required');
  const out = {...source, data:source.data.slice()}, applied = [];
  const [sx,sy] = screenOrigin;
  for (const {id,mask,active} of masks) {
    if (!appliesToCharacter(mask,active,actor.layer,actor.mapPosition)) continue;
    const [mx,my] = mask.box_top_left, [mw,mh] = mask.box_size;
    const x0 = Math.max(sx,mx), y0 = Math.max(sy,my), x1 = Math.min(sx+source.width,mx+mw), y1 = Math.min(sy+source.height,my+mh);
    if (x0 >= x1 || y0 >= y1) continue;
    const bitmap = decodeMask(mask); let outlined = 0, cleared = 0;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
      if (!bitmap[(y-my)*mw+x-mx]) continue;
      const i = (y-sy)*source.width+x-sx;
      const a = out.data[i] === source.shadowKey ? source.transparent : out.data[i];
      const next = x < x1-1 ? out.data[i+1] : source.transparent;
      const b = next === source.shadowKey ? source.transparent : next;
      const edge = drawHidden && x < x1-1 && a !== b && (a === source.transparent || b === source.transparent);
      if (edge) { out.data[i] = outlineColor; outlined++; }
      else { if(out.data[i] !== source.transparent) cleared++; out.data[i] = source.transparent; }
    }
    applied.push({id,outlined,cleared});
  }
  return {pixels:out,applied};
}

/** Split the final surface after ALL masks; the backend owns destination darkening. */
export function splitPacked(source) {
  const body = new Uint8ClampedArray(source.data.length*4), shadow = new Uint8Array(source.data.length);
  for(let i=0;i<source.data.length;i++) {
    const value=source.data[i];
    if(value===source.transparent) continue;
    if(value===source.shadowKey) {shadow[i]=255;continue;}
    const r = source.depth===16 ? value>>11&31 : value>>10&31;
    const g = source.depth===16 ? value>>5&63 : value>>5&31, b=value&31;
    body.set([(r<<3)|(r>>2),source.depth===16?(g<<2)|(g>>4):(g<<3)|(g>>2),(b<<3)|(b>>2),255],i*4);
  }
  return {width:source.width,height:source.height,body,shadow};
}
