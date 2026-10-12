import fs from 'node:fs/promises';import path from 'node:path';
import {readStoredMap,pinnedDescriptors} from '../../pipeline/src/stored-map.ts';import {compileAssetGameplay} from '../../shared/src/compile-asset-gameplay.ts';
const b=path.resolve('level-editor/work/active/croisement03/restart2'),out=path.join(b,'compiler-rebase-diagnosis-v2');await fs.mkdir(out);
for(const [name,lib]of [['canonical',path.resolve('level-editor/library')],['priorStage',b+'/combined-trio-stage-v2/library'],['candidate',b+'/combined-trio-inferred-crown-candidate-v1/browser-library-v1/library']]){
 const file=path.join(lib,'scenes/croisement03.rhlos-map.json');const doc=await readStoredMap(file,lib),assets=await pinnedDescriptors(lib,doc.assetSources,doc.sceneAssets);if(name==='canonical')for(const id of ['croisement03-east-tree-rocks','croisement03-southeast-stone-wall']){const a=assets.get(id);a.gameplay.draft.issues=[...new Set(a.gameplay.draft.issues)];}let scenerySources=[];const geometry=compileAssetGameplay(doc,assets,[0,0,1408,960],{onSceneryCompiled:s=>scenerySources=s});await fs.writeFile(path.join(out,name+'.json'),JSON.stringify({geometry,scenerySources}));console.log(name);
}
