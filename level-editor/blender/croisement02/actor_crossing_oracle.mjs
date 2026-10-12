/** Independent source-pixel oracle for explicit editor actor/effect snapshots. */
import {decodeLegacy, outlineCharacter, splitPacked} from './hidden-character-outline/outline.mjs';
import {initialMaskMembership, queryCharacterMasks} from './hidden-character-outline/mask-query.mjs';
import {compositeNativePixels} from '../../app/src/native-state-presentation.ts';

export function actorCrossingOracle(source, loop, sprite, binding) {
  const ranks=new Map();let rank=0;
  for(const [family,rows] of [['map-animation',source.level.animations],['map-patch',source.level.patches],['mission-patch',source.data.mission_patches],['civilians',source.data.civilians],['pcs_to_rescue',source.data.pcs_to_rescue],['soldiers',source.data.soldiers],['mission-target',source.data.targets]])
    rows.forEach((_,i)=>ranks.set(`${family}:${i}`,rank++));
  const draws=[...loop.draws,...(loop.orderOnly??[]).map(element=>({element,pixels:null,x:0,y:0}))].map(row=>{
    const id=`${row.element.source.kind}:${row.element.source.index}`;
    return {...row,id,rank:ranks.get(id),background:row.element.source.kind==='map-animation'&&source.level.animations[row.element.source.index].sprite.elevation===0,order:Math.fround(row.element.display_order),mapPosition:row.element.sort_position,polyline:row.element.polyline};
  });
  const [x,h,d]=sprite.position,a=source.camera.elevation_deg*Math.PI/180;
  const mapPosition=[x,d*Math.sin(a)-h*Math.cos(a)],origin=[Math.floor(x+sprite.bounds.left),Math.floor(mapPosition[1]-sprite.bounds.top)],box=[...origin,origin[0]+sprite.bounds.width,origin[1]+sprite.bounds.height];
  const masks=source.level.masks,active=initialMaskMembership(masks,[...source.level.patches,...source.data.mission_patches]);
  const indices=queryCharacterMasks(masks,active,{layer:binding.layer,mapPosition},box,[loop.background.width/64,loop.background.height/64+4]);
  const masked=outlineCharacter(decodeLegacy(sprite.frame.pixels,{depth:16,shadowKey:31}),origin,{layer:binding.layer,mapPosition},indices.map(i=>({id:`mask:${i}`,mask:masks[i],active:active[i]})),{outlineColor:binding.outlineColor,drawHidden:binding.drawHidden});
  const split=splitPacked(masked.pixels),data=new Uint8Array(split.body);
  for(let i=0;i<split.shadow.length;i++)if(split.shadow[i])data.set([0,0,255,255],i*4);
  const actor={id:binding.identity,rank:ranks.get(binding.identity),background:false,mapPosition,polyline:[],order:Math.fround(d*Math.sin(a)),pixels:{width:split.width,height:split.height,data},x:origin[0]-loop.origin[0],y:origin[1]-loop.origin[1],shadow:{rgb:[0,0,255],pixel_format:'rgb565',strength_percent:40}};
  draws.push(actor);
  const background=draws.filter(row=>row.background).sort((a,b)=>a.rank-b.rank);
  const animations=draws.filter(row=>!row.background&&row.polyline.length).sort((a,b)=>Math.min(...a.polyline.map(p=>p[1]))-Math.min(...b.polyline.map(p=>p[1]))||b.rank-a.rank);
  let remaining=draws.filter(row=>!row.background&&!row.polyline.length).sort((a,b)=>a.order-b.order||a.rank-b.rank);
  const merged=[];
  for(const animation of animations){
    const line=animation.polyline,first=line[0],last=line.at(-1),front=[];
    for(const entity of remaining){
      const [x,y]=entity.mapPosition.map(Math.fround);let behind;
      if(x<first[0])behind=y<first[1];else if(x>last[0])behind=y<last[1];else{
        let j=1;while(line[j][0]<x)j++;
        const a=line[j-1],b=line[j];behind=Math.fround(Math.fround(Math.fround(b[0]-a[0])*Math.fround(y-a[1]))-Math.fround(Math.fround(b[1]-a[1])*Math.fround(x-a[0])))<0;
      }
      (behind?merged:front).push(entity);
    }
    remaining=front;merged.push(animation);
  }
  const ordered=[...background,...merged,...remaining];
  const compose=rows=>{const output={...loop.background,data:loop.background.data.slice()};for(const row of rows)if(row.pixels)compositeNativePixels(output,row.pixels,row.x,row.y,row.shadow);return output;};
  return {ordered,actor,compose,pixels:compose(ordered),receipt:{identity:binding.identity,mapPosition,origin,masks:indices,applied:masked.applied,identities:ordered.map(row=>row.id),sourceFrame:{filename:sprite.frame.filename,profile:sprite.frame.profile,action:sprite.frame.action,direction:sprite.frame.direction,frame:sprite.frame.frame}}};
}

export function opaqueBodyOverlap(actor, effect) {
  let count=0;
  for(let y=Math.max(actor.y,effect.y);y<Math.min(actor.y+actor.pixels.height,effect.y+effect.pixels.height);y++)
    for(let x=Math.max(actor.x,effect.x);x<Math.min(actor.x+actor.pixels.width,effect.x+effect.pixels.width);x++){
      const i=((y-actor.y)*actor.pixels.width+x-actor.x)*4,j=((y-effect.y)*effect.pixels.width+x-effect.x)*4;
      if(actor.pixels.data[i+3]&&effect.pixels.data[j+3]&&!(actor.pixels.data[i]===0&&actor.pixels.data[i+1]===0&&actor.pixels.data[i+2]===255))count++;
    }
  return count;
}
