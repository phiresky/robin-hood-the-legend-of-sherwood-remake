// Execute the production pixel compositor privately; browser and gameplay remain unproved.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {NativeStatePresentation} from '../../app/src/native-state-presentation.ts';
const require=createRequire(resolve('level-editor/app/package.json'));
const {encode}=require('fast-png');
const out=resolve('level-editor/work/active/croisement01/restart2/native-controls-private-oct11-v2');
const lib=resolve('level-editor/library'),read=async p=>JSON.parse(await readFile(p,'utf8'));
const manifest=await read(join(out,'manifest.json')),resources=new Map(manifest.resources.map(r=>[r.path,r.source]));
const level=await read(join(lib,'game-data/Data/Levels/Croisement01.rhp.json')),records=[],shown=new Set();
await mkdir(join(out,'compositor-images'));
for(const r of manifest.records) {
  if(r.status!=='SOURCE_BINDINGS_PASS') continue;
  const c=await read(r.contract),mission=await read(join(lib,'game-data/Data/Levels',r.mission+'.rhm.json'));
  const p=new NativeStatePresentation(),focus=c.native.patch_states.find(s=>s.id===c.focus_patch_id);
  try {
    await p.set(c.native,{name:r.mission,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}},f=>readFile(resources.get(f.path)??join(lib,f.path)));
    const hash=()=>createHash('sha256').update(p.pixels().data).digest('hex');
    const initial=hash(),cases=[{tick:-1,rgba_sha256:initial}],images=[];
    const representative=!shown.has(focus.profile.name);shown.add(focus.profile.name);
    const save=async label=>{const image=p.pixels(),name=r.id+'-'+label+'.png';await writeFile(join(out,'compositor-images',name),encode({width:image.width,height:image.height,data:image.data,channels:4}));images.push('compositor-images/'+name);};
    if(representative) await save('initial');
    for(const tick of [...new Set([0,Math.max(0,r.terminal_tick-1),r.terminal_tick,r.terminal_tick+2])]) {
      p.seek(tick);p.setPatchState(c.focus_patch_id,'forward',tick);cases.push({tick,rgba_sha256:hash()});
    }
    if(representative) await save('terminal');
    p.seek(0);p.setPatchState(c.focus_patch_id,'initial');
    if(hash()!==initial) throw Error('Reset changed initial source composition');
    let reverseRejected=null;
    if(focus.definitive) {
      reverseRejected=false;try{p.setPatchState(c.focus_patch_id,'reverse',0);}catch{reverseRejected=true;}
      if(!reverseRejected||hash()!==initial) throw Error('Definitive reverse guard failed');
    }
    records.push({id:r.id,status:'SAMPLED_COMPOSITOR_PASS',contract_sha256:r.sha256,cases,images,reset_exact:true,definitive_reverse_rejected:reverseRejected});
  } catch(error) {records.push({id:r.id,status:'HOLD',error:String(error)});}
  finally {p.dispose();}
  console.log(r.id,records.at(-1).status);
}
await writeFile(join(out,'compositor-verification.json'),JSON.stringify({status:'SAMPLED_PRODUCTION_COMPOSITOR_ONLY',records,
  limitations:['No independent pixel parity comparison yet.','No actual editor controls or browser proof.','No physical geometry, actors, target artwork, mission scripts or gameplay proof.']},null,2)+'\n');
