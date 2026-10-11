import fs from 'node:fs/promises';
import {readStoredMap} from '../../../../pipeline/src/stored-map.ts';
import {readSceneAsset} from '../../../../pipeline/src/scene-assets.ts';
const [file,library]=process.argv.slice(2);
const raw=JSON.parse(await fs.readFile(file,'utf8'));
const document=await readStoredMap(file,library);
const refs=[...raw.assetSources,...(raw.sceneAssets??[])];
for (const ref of refs) await readSceneAsset(library,{...ref,resources:ref.resources??[]});
console.log(JSON.stringify({status:'PASS',normalStoredMapLoad:true,strictSourceCount:refs.length,objects:document.objects.length,groups:document.groups.length}));
