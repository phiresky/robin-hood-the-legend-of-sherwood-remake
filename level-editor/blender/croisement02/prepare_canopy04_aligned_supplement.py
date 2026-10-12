"""Prepare each remaining complete canopy group as an independent bounded review."""
from pathlib import Path
import json,hashlib,subprocess,shutil
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';SOURCE=B/'canopy00-junction-diagnostics-playerfix-oct12-v1';RUNNER=B/'canopy04-complete-review-playerfix-http-v2';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();catalog={x['asset_id']:x for x in json.loads((B/'restart101-five-combined-canopy-browser-v1/request.json').read_text())['rows']};catalog.update({x['asset_id']:x for x in json.loads((B/'canopy34-complete-review-oct12-v1/request.json').read_text())['rows']})
def walk(value):
 if isinstance(value,dict):
  if isinstance(value.get('path'),str)and isinstance(value.get('sha256'),str):
   p=Path(value['path']).resolve();assert sha(p)==value['sha256'],str(p);value['path']=str(p)
  for x in value.values():walk(x)
 elif isinstance(value,list):
  for x in value:walk(x)
def collect(value):
 if isinstance(value,dict):
  if isinstance(value.get('path'),str)and isinstance(value.get('sha256'),str):yield value
  for x in value.values():yield from collect(x)
 elif isinstance(value,list):
  for x in value:yield from collect(x)
for group in [4]:
 out=B/f'canopy{group:02}-aligned-junction-supplement-oct12-v1';out.mkdir(exist_ok=False);row=catalog[f'canopy-group-{group:02}'];bundle=json.loads(Path(row['export']['path']).read_text());row['tree_ids']=[x['asset_id'].rsplit('-',1)[1]for x in bundle['assets']];row['native_source_frames']=json.loads((B/f'restart80-canopy{group:02}-dense-motion-v1/report.json').read_text())['source_frames'];row['static_sources']=[x['static_source']for x in bundle['assets']];request={'status':'PRIVATE_COMPLETE_GROUP_MOTION_UNAPPROVED','rows':[row]};walk(request);(out/'request.json').write_text(json.dumps(request,indent=2)+'\n')
 (out/'run.mjs').write_text((RUNNER/'run.mjs').read_text().replace(RUNNER.name,out.name));(out/'build.mjs').write_text((SOURCE/'build.mjs').read_text().replace(SOURCE.name,out.name));s=(SOURCE/'proof.ts').read_text().replace('batch.native_source_frames','request.native_source_frames').replace("for(const id of ['43','45','46'])",'for(const id of request.tree_ids)').replace('canopy00-native-bbox','${request.asset_id}-native-bbox').replace('canopy00-tree-','${request.asset_id}-tree-')
 s=s.replace("for(const mode of ['transparent','cutaway']){", "for(const mode of (tick===0?['transparent']:['transparent','cutaway'])){")
 s=s.replace('await nativeView(tick,object);if(tick===0||tick===28)await contact(tick,object);','await nativeView(tick,object);if(tick===0||tick===28)await contact(tick,object);')
 s=s.replace('Supplemental diagnostic-only:15exact native-bbox phase views and24isolated role-color/cutaway junction views.','Supplement to existing actual34views:15exact native-bbox phase views,transparent junctions at0/7,and cutaway at7.')
 (out/'proof.ts').write_text(s);subprocess.run(['node',str(out/'build.mjs')],check=True);subprocess.run(['node','--check',str(out/'run.mjs')],check=True);pins=json.loads((out/'source-pins.json').read_text());snapshot={}
 for path,digest in list(pins.items()):
  source=Path(path);assert sha(source)==digest;relative=source.relative_to(R)if source.is_absolute()else source;dest=out/'runtime-snapshot'/relative;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,dest);snapshot[str(dest)]=digest
 pins.update(snapshot)
 for pin in collect(request):pins[pin['path']]=pin['sha256']
 for name in ['request.json','run.mjs','build.mjs','proof.ts','proof.js','index.html']:pins[str(out/name)]=sha(out/name)
 (out/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');handoff={'status':'PREPARED_NOT_LAUNCHED','group':group,'trees':row['tree_ids'],'runner':{'path':str(out/'run.mjs'),'sha256':sha(out/'run.mjs')},'pins':{'path':str(out/'source-pins.json'),'sha256':sha(out/'source-pins.json')},'captures':15+6*len(row['tree_ids']),'production_ticks':113,'source_camera':'Original direction, exact nativebbox dimensions and1sourcepixel/unit;15saved phaseviews includeclosure. Other7directions and actual junctions retained in previous executed packet.','junctions':'Per-tree isolated original/reverse transparentrole0/7,cutawayrole7. Magentafixedwood/cyanmovingcrown. Diagnostic materials restored after eachcapture.','player':{'commit':'9a45b85fe','sha256':sha(R/'level-editor/app/src/state-appearance-player.ts')},'limits':'Motion remains unapproved; all saved pictures must be reviewed. Leaf-card surfaces need not be welded to implicit hidden twigs. No blanketcrossgroup clearanceclaim.'};(out/'handoff.json').write_text(json.dumps(handoff,indent=2)+'\n');print(group,handoff['captures'],sha(out/'handoff.json'),flush=True)
