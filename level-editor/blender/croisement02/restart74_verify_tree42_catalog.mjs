import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {parseStoredMap} from '../../shared/src/stored-level.ts';
import {loadMissionStatePreview} from '../../app/src/mission-state-catalog.ts';
import {verifyStaticReplacementRows} from '../../shared/src/state-delivery.ts';
const root=resolve('.'),lib=join(root,'level-editor/library'),stage=join(root,'level-editor/work/active/croisement02/restart62-tree42-catalog-stage-v2');
const read=async p=>JSON.parse(await readFile(p)),sha=b=>createHash('sha256').update(b).digest('hex');
const plan=await read(join(stage,'plan.json'));assert.equal(plan.catalog_additions.length,1);
const mappings=new Map();for(const row of plan.files){assert.equal(sha(await readFile(row.staged.path)),row.staged.sha256);mappings.set(row.destination,row.staged.path)}
const stored=await read(join(lib,'scenes/croisement02.rhlos-map.json')),descriptors=new Map();for(const ref of [...stored.assetSources??[],...stored.sceneAssets??[]])descriptors.set(ref.id,await read(join(lib,ref.descriptor)));const document=parseStoredMap(stored,descriptors);
function directory(prefix=''){return{async getDirectoryHandle(n){return directory(prefix+n+'/')},async getFileHandle(n){return{async getFile(){return new File([await readFile(mappings.get(prefix+n)??join(lib,prefix+n))],n)}}}}}
for(const entry of plan.catalog_additions){const loaded=await loadMissionStatePreview(directory(),entry);assert.equal(loaded.kind,'native-loop');assert.equal(loaded.contract.focus_element_id,'animation1');verifyStaticReplacementRows(loaded.contract.static_replacements,document);assert.equal(loaded.contract.static_replacements.length,2)}
console.log(JSON.stringify({status:'PASS_CURRENT_TREE42_CATALOG_AND_STATIC_REPLACEMENTS',entries:1,static_parts:2,canonical_writes:false}));
