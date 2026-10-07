import { appliesToCharacter } from '../native-character-masks/character-masks.mjs';

/** Pre-script load baseline only. References index within their own layer. */
export function initialMaskMembership(masks, orderedPatches) {
  const layers = new Map(), active = masks.map(()=>true);
  masks.forEach((m,i)=> {if(!layers.has(m.layer))layers.set(m.layer,[]);layers.get(m.layer).push(i);});
  for(const patch of orderedPatches) for(const [key,value] of [['old_masks',true],['new_masks',false]])
    for(const ref of patch[key]) {
      const index=layers.get(ref.layer)?.[ref.index];
      if(index===undefined)throw Error('Unresolved layer-local mask reference');
      active[index]=value;
    }
  return active;
}

/** Preserve row-major cell traversal and first encounter, then precise filtering. */
export function queryCharacterMasks(masks, active, actor, box, gridSize) {
  if(masks.length!==active.length || active.some(x=>typeof x!=='boolean') ||
    box.length!==4 || !box.every(Number.isFinite) || box[2]<box[0] || box[3]<box[1] ||
    gridSize.length!==2 || !gridSize.every(x=>Number.isInteger(x)&&x>0)) throw Error('Invalid mask query');
  const cells=rect=>rect.map((x,i)=>Math.max(0,Math.min(gridSize[i%2]-1,Math.trunc(x)>>6)));
  const bounds=masks.map(m=>{const [x,y]=m.box_top_left,[w,h]=m.box_size;return [x,y,x+w,y+h];});
  const grids=bounds.map(cells),[x0,y0,x1,y1]=cells(box),seen=new Set(),ordered=[];
  for(let y=y0;y<=y1;y++)for(let x=x0;x<=x1;x++)for(let i=0;i<masks.length;i++) {
    const m=masks[i],[a,b,c,d]=grids[i];
    if(m.layer===actor.layer && active[i] && (m.mask_type&1) && x>=a && x<=c && y>=b && y<=d && !seen.has(i)) {
      seen.add(i);ordered.push(i);
    }
  }
  return ordered.filter(i=>{
    const b=bounds[i];
    return b[0]<=box[2]&&b[2]>=box[0]&&b[1]<=box[3]&&b[3]>=box[1]&&
      appliesToCharacter(masks[i],active[i],actor.layer,actor.mapPosition);
  });
}
