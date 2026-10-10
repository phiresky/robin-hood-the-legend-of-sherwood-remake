/** Stage the complete native canopy animation through existing authoring contracts. */
import fs from 'node:fs/promises';import path from 'node:path';import crypto from 'node:crypto';import assert from 'node:assert/strict';
import {recoverSceneryAnimation} from '../../pipeline/src/recover-scenery-animation.ts';
import {compileSceneryAnimation} from '../../shared/src/compile-scenery-animation.ts';
import {writeSceneryAnimationAssets} from '../../pipeline/src/author-scenery-animation-assets.ts';
const root=process.cwd(),b=path.join(root,'level-editor/work/croisement03-refinement/restart2'),out=path.join(b,'combined-trio-stage-v2');await fs.mkdir(out,{recursive:true});
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),read=async p=>JSON.parse(await fs.readFile(p,'utf8'));
const levelPath=path.join(b,'../baseline/Croisement03.rhp.json'),level=await read(levelPath),source=level.animations[5];assert.equal(source.sprite.profile_name,'Croisement03 - Arbre06');
const bank=path.join(root,'datadirs/fullgame_gog_hackable/Data/Animations/Day/Treecr03.rhs.d'),manifestPath=path.join(bank,'manifest.json'),manifest=await read(manifestPath),profile=manifest.profiles.find(p=>p.name===source.sprite.profile_name);assert.ok(profile);assert.equal(profile.rows.length,1);assert.equal(profile.rows[0].frames.length,14);assert.ok(profile.rows[0].frames.every(f=>f.delay===3));
const directory='scenery/croisement03-native-arbre06.rhs.d',files=new Map(),frameProof=[];for(const row of profile.rows)for(const [i,f] of row.frames.entries()){
 const src=path.join(bank,profile.name,row.path,f.file),bytes=await fs.readFile(src),dest=`${directory}/${row.path}/${f.file}`;files.set(dest,bytes);frameProof.push({index:i,source:src,sha256:sha(bytes),path:dest,delay:f.delay,durationTicks:f.delay+1,offset:[f.offset_x,f.offset_y]});
}
files.set(`${directory}/manifest.json`,Buffer.from(JSON.stringify({...manifest,profiles:[profile]},null,2)+'\n'));
const s=source.sprite,origin=[s.position_x+profile.center_x,s.position_y+profile.center_y+s.elevation,s.elevation],animation=recoverSceneryAnimation(source,profile,{id:'native-arbre06',node:'scenery-animation',anchor:origin,resourceDirectory:directory},p=>p.map((v,i)=>v-origin[i]));
const compiled=compileSceneryAnimation(animation,(_n,p)=>p.map((v,i)=>v+origin[i]));assert.deepEqual(compiled,source);
const broken=structuredClone(animation);broken.anchor[0]++;assert.notDeepEqual(compileSceneryAnimation(broken,(_n,p)=>p.map((v,i)=>v+origin[i])),source);
const id='croisement03-native-arbre06',recipe={version:1,entries:[{id,name:'North woodland shared native canopy animation',map:'Croisement03',origin,animations:[animation],resources:[...files].map(([path,bytes])=>({path,sha256:sha(bytes)}))}]};
const assets=await writeSceneryAnimationAssets(recipe,async n=>{assert.ok(files.has(n));return files.get(n);},path.join(out,'scenery-library'));
await fs.writeFile(path.join(out,'animation-contract.json'),JSON.stringify({status:'PASS exact native round-trip and pinned 14-frame resource contract; browser phase review pending',sourceAnimationIndex:5,sourceAnimation:source,compiledAnimation:compiled,origin,fullCycleTicks:frameProof.reduce((n,f)=>n+f.durationTicks,0),sourceLevelSha256:sha(await fs.readFile(levelPath)),sourceBankManifestSha256:sha(await fs.readFile(manifestPath)),frameProof,assetSources:assets.assetSources,objects:assets.objects,negativeAnchorCheck:true,scope:'One complete shared canopy owner; no fragment splitting, inferred motion or changes to other animations. Tree13 static map foliage remains distinct.'},null,2)+'\n');console.log(JSON.stringify({assets:assets.assetSources.length,frames:frameProof.length,cycleTicks:56,origin}));
