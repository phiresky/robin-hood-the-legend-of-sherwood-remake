// Small real-compositor crops for independent inspection of the initial-context correction.
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import assert from 'node:assert/strict';
import {NativeStatePresentation} from '../../app/src/native-state-presentation.ts';
const require=createRequire(resolve('level-editor/app/package.json'));
const {encode}=await import(require.resolve('fast-png'));
const lib=resolve('level-editor/library'),stage=resolve('level-editor/work/croisement02-refinement/restart2-state/restart16-initial-context-v1');
const read=async p=>JSON.parse(await readFile(p,'utf8')),sha=b=>createHash('sha256').update(b).digest('hex');
const manifest=await read(join(stage,'manifest.json')),level=await read(join(lib,'game-data/Data/Levels/Croisement02.rhp.json')),records=[];
for(const row of manifest.records.filter(r=>r.id.endsWith('-log-trap'))){
 const before=await read(join(lib,row.path)),after=await read(join(stage,'library',row.path));
 const source={name:row.mission,data:await read(join(lib,`game-data/Data/Levels/${row.mission}.rhm.json`)),level,camera:{kind:'oblique-orthographic',elevation_deg:35}};
 const players=[new NativeStatePresentation(),new NativeStatePresentation()];
 try{
  for(const [i,c]of [before,after].entries())await players[i].set(c.native,source,async r=>{const b=await readFile(join(lib,r.path));assert.equal(sha(b),r.sha256);return b;});
  const pixels=players.map(p=>p.pixels()),states=after.native.patch_states.filter(s=>row.added_context_ids.includes(s.id));
  const width=512,height=states.length*160,data=new Uint8Array(width*height*4);for(let i=0;i<data.length;i+=4){data[i]=data[i+1]=data[i+2]=30;data[i+3]=255;}
  const labels=[];
  for(const [rowIndex,s]of states.entries()){
   const f=s.initial[0],cx=s.display_position[0]+f.offset[0]+f.width/2-after.native.origin[0],cy=s.display_position[1]+f.offset[1]+f.height/2-after.native.origin[1];
   const scale=f.width<112&&f.height<64?2:1;labels.push({row:rowIndex,patch:s.id,scale,frame_size:[f.width,f.height],layer:s.layer,elevation:s.elevation});
   for(let column=0;column<2;column++)for(let y=16;y<160;y++)for(let x=0;x<256;x++){
    const sx=Math.floor(cx+(x-128)/scale),sy=Math.floor(cy+(y-88)/scale),p=pixels[column];if(sx<0||sy<0||sx>=p.width||sy>=p.height)continue;
    const src=(sy*p.width+sx)*4,dst=((rowIndex*160+y)*width+column*256+x)*4;data.set(p.data.subarray(src,src+4),dst);
   }
  }
  const image=`${row.mission}-initial-before-after.png`,bytes=encode({width,height,channels:4,data});assert.ok(bytes.length<2*1024*1024);
  await writeFile(join(stage,image),bytes,{flag:'wx'});records.push({mission:row.mission,image,sha256:sha(bytes),bytes:bytes.length,columns:['installed initial','private corrected initial'],rows:labels});
 }finally{players.forEach(p=>p.dispose());}
}
await writeFile(join(stage,'visual-evidence.json'),JSON.stringify({status:'REAL_NATIVE_COMPOSITOR_INITIAL_CROPS',records,scope:'Crops from complete ordered CPU composites, not post-script mission gameplay.'},null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify(records.map(r=>({mission:r.mission,bytes:r.bytes,rows:r.rows.length}))));
