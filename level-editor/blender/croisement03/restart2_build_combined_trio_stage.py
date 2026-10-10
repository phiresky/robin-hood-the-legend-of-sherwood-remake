"""Build a guarded private map with static trio, approved ground and one native canopy owner."""
import copy,hashlib,json,shutil
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';O=B/'combined-trio-stage-v2';LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 library=O/'library';assert not library.exists();library.mkdir();pins={};mappath=LIB/'scenes/croisement03.rhlos-map.json';before=read(mappath);mapdoc=copy.deepcopy(before);pins[str(mappath)]=sha(mappath);metadata=read(O/'metadata/receipt.json');assert metadata['live_map_sha256']==sha(mappath)
 # Unmodified library resources are read through explicit links; no writer targets them.
 (library/'3d-assets').mkdir();(library/'3d-assets/croisement03').mkdir()
 for p in (LIB/'3d-assets').iterdir():
  if p.name!='croisement03':(library/'3d-assets'/p.name).symlink_to(p.resolve(),target_is_directory=p.is_dir())
 for p in (LIB/'3d-assets/croisement03').iterdir():
  if p.name!='croisement03-terrain':(library/'3d-assets/croisement03'/p.name).symlink_to(p.resolve(),target_is_directory=p.is_dir())
 for p in LIB.iterdir():
  if p.name not in ['3d-assets','scenes','scenery']:(library/p.name).symlink_to(p.resolve(),target_is_directory=p.is_dir())
 retired=set();rows=[]
 for tree in [12,13,14]:
  asset=f'croisement03-tree-{tree}';meta=read(O/f'metadata/{asset}/asset-metadata-proposal.json');parity=read(O/f'metadata/{asset}/parity.json');assert parity['maximum_obstacle_world_error']==0
  for p,h in parity['protectedFiles'].items():assert sha(p)==h;pins[p]=h
  if tree==13:
   src=B/'tree13-exact-export-v2/model.glb';assert sha(src)=='5cda7368afa8b8b884b040e6c2effc28b4c4c4ca78b7e7ca23b11469050654ad';approval=B/'user-approval-v14/croisement03-tree-13-local75.json';assert read(approval)['model_sha256']=='ef3f45a211714993959e420ff99fb3b2c709068971861ce6336c9a010f4cc553';pins[str(approval)]=sha(approval)
  else:
   base=B/f'static-pair-publication-audit-v1/static-body-v1/tree{tree}';src=base/'model.glb';static=read(base/'asset.json');assert static['source_origin_scene']==meta['source_origin_scene'];role=f'foliage-{asset}-dynamic-frame0-provenance';meta['parts']=[p for p in meta['parts'] if p['node']!=role];meta['components']=[p for p in meta['components'] if p['source_node']!=role];meta['bounds_local_scene']=static['bounds_local_scene'];assert len([p for p in meta['parts'] if p.get('scenery')])==1
  dest=library/f'3d-assets/croisement03/{asset}';dest.mkdir();shutil.copyfile(src,dest/'model.glb');meta['resources']=[];write(dest/'asset.json',meta);pins[str(src)]=sha(src)
  oldids={r['prior_asset'] for r in parity['fragments']};retired|=oldids;mapdoc['assetSources']=[r for r in mapdoc['assetSources'] if r['id'] not in oldids];oldplacements=[p for p in mapdoc['placements'] if any(x in oldids for x in p['assets'])];assert all(len(p['assets'])==1 for p in oldplacements);mapdoc['placements']=[p for p in mapdoc['placements'] if p not in oldplacements]
  ref=dict(id=asset,descriptor=str((dest/'asset.json').relative_to(library)),descriptor_sha256=sha(dest/'asset.json'),model=str((dest/'model.glb').relative_to(library)),model_sha256=sha(dest/'model.glb'),model_scene=meta['model_scene']);mapdoc['assetSources'].append(ref);mapdoc['placements'].append(dict(id=asset,transform=parity['proposed_transform'],assets=[asset]));rows.append(dict(asset=asset,retired=sorted(oldids),ref=ref,transform=parity['proposed_transform'],source=str(src)))
 ground=B/'approved-trio-ground-integration-v1';gr=read(ground/'report.json');assert sha(ground/'model.glb')==gr['model_sha256'];dest=library/'3d-assets/croisement03/croisement03-terrain';dest.mkdir()
 for name in ['model.glb','asset.json']:shutil.copyfile(ground/name,dest/name);pins[str(ground/name)]=sha(ground/name)
 gs=next(r for r in mapdoc['sceneAssets'] if r['id']=='croisement03-terrain');gs['model_sha256']=sha(dest/'model.glb');gs['descriptor_sha256']=sha(dest/'asset.json')
 animation=read(O/'animation-contract.json');ar=animation['assetSources'][0];ap=animation['objects'][0];assert all('native-arbre06' not in x['id'] for x in before['assetSources']);shutil.copytree(O/'scenery-library/3d-assets/croisement03-native-arbre06',library/'3d-assets/croisement03-native-arbre06');shutil.copytree(O/'scenery-library/scenery',library/'scenery');mapdoc['assetSources'].append(ar);mapdoc['placements'].append(dict(id=ar['id'],transform=ap['transform'],assets=[ar['id']]))
 # Metadata, unrelated references/placements, mission states and camera are immutable.
 for k in before:
  if k not in ['assetSources','placements','sceneAssets']:assert mapdoc[k]==before[k]
 assert [r for r in mapdoc['assetSources'] if r['id'] in {x['id'] for x in before['assetSources']}]==[r for r in before['assetSources'] if r['id'] not in retired]
 assert [p for p in mapdoc['placements'] if p['id'] in {x['id'] for x in before['placements']}]==[p for p in before['placements'] if not any(a in retired for a in p['assets'])]
 (library/'scenes').mkdir();write(library/'scenes/croisement03.rhlos-map.json',mapdoc)
 source=np.asarray(Image.open(B.parent/'baseline/covered.png').convert('RGBA'));local=np.asarray(Image.open(B/'tree13-canopy-context-v1/provisional75-excluding-known-bark.png').convert('RGBA'));domain=np.zeros(source.shape[:2],bool);domain[:57,1006:1132]=local[:,:,3]>0;assert domain.sum()==3244;assert np.array_equal(local[local[:,:,3]>0],source[:57,1006:1132][local[:,:,3]>0]);fillmask=np.asarray(Image.open(ground.parent/'approved-hub-textures-v1/trio-ground/correction-v3-region-guide/experiment/mask.png').convert('RGBA'))[:,:,3]==0;assert not (domain&~fillmask).any()
 disjoint=[]
 for tree in [12,14]:
  p=B/f'trio-tree-integration-v1/static-leaf-source-proposal-v2/tree{tree}-proposed-source-rgba.png';d=np.asarray(Image.open(p).convert('RGBA'))[:,:,3]>0;assert not (d&domain).any();disjoint.append(dict(tree=tree,overlap_with_static75=0))
 proof=dict(status='PASS static source identity; joint runtime first-hit still pending',tree13_static_pixels=3244,static_source_rgb_exact=True,all_static75_ground_duplicates_filled=True,other_static_domains=disjoint,source_semantics='Pixels are from the covered static map, independent of separately composited animation frames. Mask overlap does not change image provenance. Existing approved geometry/appearance and full Tree13 GLB remain unchanged.',animation_semantics='One complete native Arbre06 owner, exact original top-left/center/elevation/polyline/flags and all 14 source frames; 56 ticks per cycle. Dynamic frame0 proxy roles are absent from Tree12/14 selected static scenes.')
 write(O/'tree13-static-ownership.json',proof);write(O/'staging-receipt.json',dict(status='PRIVATE CONCRETE STAGE; strict loader/compile/browser checks pending; not executable publication',library=str(library),map=str(library/'scenes/croisement03.rhlos-map.json'),map_sha256=sha(library/'scenes/croisement03.rhlos-map.json'),rows=rows,retired_assets=sorted(retired),native_animation_asset=ar,source_metadata_camera_mission_exact=True,pins=pins,remaining=['Strict stored-map and model-resource validation','Compile exact scenery animation and verify full source profile resource binding','Joint source/oblique/all-phase runtime first-hit including fern reservations','Private derivative/index generation with current source guards','Root-reviewed promotion and rollback plan after browser PASS']));assert all(sha(p)==h for p,h in pins.items());print(O/'staging-receipt.json')
if __name__=='__main__':main()
