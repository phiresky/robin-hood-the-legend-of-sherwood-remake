"""Guarded publication of approved decorative Tree03 and exact native017 paint transfer only."""
import copy,json,os,shutil,sys,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
from asset_index import validate_asset_index
R=ROOT/'level-editor/work/croisement01-refinement/restart2';stage=R/'tree03-integration-batch-v1';library=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 revision=sys.argv[1];assert revision.startswith('full-editor-private-v') and Path(revision).name==revision
 folder=stage/revision;result=read(folder/'result.json');config=read(folder/'config.json');assert result['status']=='PASS' and result['mapGroups']==60 and result['mapParts']==98;assert result['filesHashVerified']==len(config['files']);assert len(result['selectionChecks'])==60;assert set(result['insertedAssets'])=={a.get('inserted_id',a['id']) for a in config['expected']['assets'] if a.get('editor_usage')!='map-background'}
 for path,digest in config['protected_live_files'].items():assert sha(Path(path))==digest,path
 for path,digest in read(folder/'checker-provenance.json').items():assert sha(ROOT/path)==digest,path
 proof=read(stage/'scene-splice-proof.json');scene=library/'scenes/croisement01.rhlos-map.json';index=library/'3d-assets/index.json';assert sha(scene)==proof['live_scene_sha256'];assert sha(stage/scene.name)==proof['staged_scene_sha256'];assert next(f['sha256'] for f in config['files'] if f['path']=='scenes/croisement01.rhlos-map.json')==proof['staged_scene_sha256']
 asset='croisement01-tree-03';residual='croisement01-group-005';scoped={asset,residual};partition=read(R/'tree03-visual-transfer-v1/proof.json');base=R/'tree03-integration-v2';case=R/'approved-tree03-fill-v1'/asset;visual=read(base/'glb-review-v1/root-review.json');assert visual['status']=='scoped exported appearance PASS';assert visual['source_glb_sha256']==proof['assets'][asset]['model_sha256']
 for path,digest in visual['files'].items():assert sha(Path(path))==digest,path
 assert sha(R/'tree03-visual-transfer-v1/proof.json')==proof['transfer_proof_sha256'];assert partition['source_pixels']==46 and partition['geometry_buffers_exact'] and partition['gameplay_and_part_metadata_exact'] and partition['outside_domain_rgba_exact']
 root=read(folder/'root-review.json');assert root['status']=='PASS scoped private full Editor';assert root['result_sha256']==sha(folder/'result.json')
 for name,digest in root['screenshots'].items():assert sha(folder/name)==digest,name

 export=read(base/'export-proof.json');assert export['geometry']['geometry_verified'];assert export['approved_user_decision_sha256']==sha(case/'user-texture-decision.json')
 target_base=library/'3d-assets/croisement01';assert not (target_base/asset).exists();assert sha(target_base/residual/'model.glb')==partition['source_model_sha256'];assert sha(target_base/residual/'asset.json')==partition['source_descriptor_sha256']
 for identifier in scoped:
  for name,key in [('model.glb','model_sha256'),('asset.json','descriptor_sha256')]:assert sha(stage/'assets'/identifier/name)==proof['assets'][identifier][key]
 assert sha(stage/'assets'/residual/'model.glb')==partition['output_model_sha256'];assert sha(stage/'assets'/residual/'asset.json')==partition['output_descriptor_sha256']
 atlas_name=partition['atlas_sha256']+'.png';atlas_source=R/'tree03-visual-transfer-v1/3d-assets/blobs'/atlas_name;assert sha(atlas_source)==partition['atlas_sha256'];atlas_target=library/'3d-assets/blobs'/atlas_name
 if atlas_target.exists():assert sha(atlas_target)==partition['atlas_sha256']
 before=read(index);index_hash=sha(index);entries=read(stage/'assets/index.json')['assets'];assert {e['id'] for e in entries}==scoped;replacements={}
 for row in entries:
  entry=copy.deepcopy(row)
  for key in ['model','descriptor']:entry[key]='croisement01/'+entry[key]
  replacements[entry['id']]=entry
 assert sum(e['id']==residual for e in before['assets'])==1 and not any(e['id']==asset for e in before['assets']);updated=copy.deepcopy(before);updated['assets']=[replacements[e['id']] if e['id']==residual else e for e in before['assets']]+[replacements[asset]];assert [e for e in updated['assets'] if e['id'] not in scoped]==[e for e in before['assets'] if e['id'] not in scoped]
 if '--check' in sys.argv:print(json.dumps(dict(status='PASS read-only promotion preflight',scene_sha256=sha(scene),index_sha256=index_hash)));return
 if shutil.disk_usage(stage).free<25*1024**3:raise ValueError('Disk floor25GiB')
 backup=stage/'publication-backup-v1';backup.mkdir(exist_ok=False);shutil.copy2(scene,backup/scene.name);os.link(index,backup/'asset-index.json');shutil.copytree(target_base/residual,backup/residual)
 protected={str(p.relative_to(library)):sha(p) for p in target_base.rglob('*') if p.is_file() and residual not in p.parts};write(backup/'receipt.json',dict(scene_sha256=sha(scene),index_sha256=index_hash,protected_existing_assets=protected,rollback='Restore only scoped scene/residual after current-hash checks; coordinate index rollback to preserve later unrelated entries. Index backup is immutable and source index is replaced atomically.'))
 assert sha(index)==index_hash and sha(scene)==proof['live_scene_sha256'];installed={}
 if not atlas_target.exists():
  fd,name=tempfile.mkstemp(prefix='.tree03-atlas-',dir=atlas_target.parent);os.close(fd);tmp=Path(name);shutil.copy2(atlas_source,tmp);assert sha(tmp)==partition['atlas_sha256'];os.replace(tmp,atlas_target)
 installed[str(atlas_target.relative_to(library))]=sha(atlas_target)
 for identifier in [asset,residual]:
  source=stage/'assets'/identifier;target=target_base/identifier
  if identifier==asset:target.mkdir()
  for file in source.iterdir():
   assert file.is_file();fd,name=tempfile.mkstemp(prefix='.tree03-',dir=target);os.close(fd);tmp=Path(name);shutil.copy2(file,tmp);assert sha(tmp)==sha(file);os.replace(tmp,target/file.name);installed[str((target/file.name).relative_to(library))]=sha(target/file.name)
 # Old preview/lossy files no longer bind the residual and must not be rediscovered.
 archived={};archive=backup/'retired-derived-files';archive.mkdir()
 for name in ['preview.glb','preview.glb.receipt.json','lossy.glb','lossy.glb.receipt.json']:
  path=target_base/residual/name
  if path.exists():
   digest=sha(path);assert digest==sha(backup/residual/name);os.replace(path,archive/name);assert sha(archive/name)==digest;archived[str(path.relative_to(library))]=dict(archive=str(archive/name),sha256=digest)
 validate_asset_index(library/'3d-assets',updated)
 for path,digest in protected.items():assert sha(library/path)==digest,path
 assert sha(index)==index_hash and sha(scene)==proof['live_scene_sha256'];temp=index.with_name('.index-croisement01-tree03.json');write(temp,updated);os.replace(temp,index);temp=scene.with_name('.croisement01-tree03.json');shutil.copy2(stage/scene.name,temp);os.replace(temp,scene);assert sha(scene)==proof['staged_scene_sha256']
 write(stage/'publication.json',dict(status='installed; bounded live proof pending',scene_sha256=sha(scene),index_sha256=sha(index),published_files=installed,retired_derivatives=archived,private_full_editor=dict(path=str(folder/'result.json'),sha256=sha(folder/'result.json'),config_sha256=sha(folder/'config.json'),runtime_source_sha256=sha(folder/'runtime-source-provenance.json')),backup=str(backup),unchanged_other_placements=proof['unchanged_other_placements'],unrelated_palette_entries_preserved=len(before['assets'])-1,scope=proof['scope']));print(stage/'publication.json')
if __name__=='__main__':main()
