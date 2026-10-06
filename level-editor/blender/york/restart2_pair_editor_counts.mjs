/** Derive editor expectations through the same descriptor-backed storage parser. */
import fs from 'node:fs';
import {parseStoredMap} from '../../shared/src/stored-level.ts';
const path=process.argv[2];
if(!path)throw Error('Supply private editor config');
const config=JSON.parse(fs.readFileSync(path));
const files=new Map(config.files.map(file=>[file.path,file.url.slice(5)]));
const stored=JSON.parse(fs.readFileSync(files.get('scenes/york.rhlos-map.json')));
const descriptors=new Map([...stored.assetSources,...stored.sceneAssets].map(reference=>[
 reference.id,JSON.parse(fs.readFileSync(files.get(reference.descriptor)))
]));
const document=parseStoredMap(stored,descriptors);
Object.assign(config.expected,{groups:document.groups.length,parts:document.objects.length,
 ungrouped_parts:document.objects.filter(part=>!part.group).length});
fs.writeFileSync(path,JSON.stringify(config,null,2)+'\n');
console.log(JSON.stringify({groups:config.expected.groups,parts:config.expected.parts,ungrouped_parts:config.expected.ungrouped_parts}));
