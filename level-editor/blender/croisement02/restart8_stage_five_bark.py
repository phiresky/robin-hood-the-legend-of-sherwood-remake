"""Prepare an isolated, reversible five-bark delta; never writes the live library."""
from pathlib import Path
import argparse,copy,hashlib,json,os,sys
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from asset_index import write_asset_index,discover_asset_index
from promote_staged_publication import library_lock,check_gameplay_preserved
from lossy_assets import verify_derivatives,read_glb
O=ROOT/'level-editor/work/croisement02-refinement';B=O/'restart8-five-bark-approved-export-v1';L=ROOT/'level-editor/library'
NUMBERS=[18,24,38,39,45]

def read(p):return json.loads(p.read_text())
def sha(p):
 if not p.exists():return None
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def link(src,dst):
 dst.parent.mkdir(parents=True,exist_ok=True)
 if dst.exists():assert sha(src)==sha(dst)
 else:os.link(src,dst)
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--expected-map-sha256',required=True);parser.add_argument('--stage',type=Path,required=True);args=parser.parse_args();EXPECTED_MAP=args.expected_map_sha256;S=args.stage.resolve();assert len(EXPECTED_MAP)==64 and all(c in '0123456789abcdef'for c in EXPECTED_MAP);assert S.parent==O.resolve() and S.name.startswith('restart8-five-bark-publication-delta-');assert not S.exists()
 A=S/'map-assets';root=A/'3d-assets';live_map=L/'scenes/croisement02.rhlos-map.json';assert sha(live_map)==EXPECTED_MAP;S.mkdir()
 approval=O/'restart3-review-batches/batch-v16/user-approval.json';assert sha(approval)=='e350fc5e83775a6aec133c9346371b4699955081267d625dfe30a6813943e941'
 with library_lock(L):
  baseline=read(live_map);index=read(L/'3d-assets/index.json');index_sha=sha(L/'3d-assets/index.json');byid={x['id']:x for x in index['assets']};target_ids={f'croisement02-tree-{n}'for n in NUMBERS};records=[];deltas=[];pins=[];mapping={}
  refs=baseline['assetSources']+baseline['sceneAssets'];assert len({x['id']for x in refs})==len(refs)
  for ref in refs:
   entry=byid[ref['id']];src=L/'3d-assets'/entry['descriptor'];descriptor=read(src);folder=root/Path(entry['descriptor']).parent
   assert sha(src)==ref['descriptor_sha256'];assert sha(L/ref['model'])==ref['model_sha256']
   for resource in descriptor.get('resources',[]):
    source=L/resource['path'];assert sha(source)==resource['sha256'];link(source,A/resource['path'])
   changed=ref['id']in target_ids
   for file in src.parent.rglob('*'):
    if file.is_file()and not(changed and file.name in ['model.glb','lossy.glb','lossy.glb.receipt.json','preview.glb','preview.glb.receipt.json']):link(file,folder/file.relative_to(src.parent))
   if changed:
    n=int(ref['id'].rsplit('-',1)[1]);work=B/f'tree-{n}-v1';e=read(work/'export.json');guard=read(work/'source-surface-guard.json');rootreview=read(work/'root-review-v4.json');assert 'PASS'in rootreview['status'];assert guard['source_model_unchanged']and guard['all_exported_positions_on_original_triangle']and guard['all_shader_uvs_match_correct_original_layer'];assert e['approval_sha256']==sha(approval)
    assert descriptor['source_origin_scene']==e['pivot'];assert sorted(x['node']for x in descriptor['parts'])==e['parts']
    native=work/'delivery-v4/model.glb';delivery=work/'delivery-v4/lossless.glb';norm=read(work/'live-crown-sampler-guard-v1.json');assert sha(native)==norm['files'][0]['output_sha256']and sha(delivery)==norm['files'][1]['output_sha256'];assert rootreview['delivery_sha256']==sha(delivery) and rootreview['source_model_sha256']==e['source_sha256'] and rootreview['self_review_sha256']==sha(work/'self-review-v4.json');selfreview=read(work/'self-review-v4.json');assert all(sha(ROOT/path)==digest for path,digest in selfreview['files'].items());assert all(all(row[key]for key in ['binary_chunks_exact','all_geometry_uv_images_exact','all_bark_materials_exact','all_other_materials_exact','crown_materials_match_frozen_live_semantics'])for row in norm['files']);doc,_,_=read_glb(native);assert len(doc['scenes'])==1 and doc['scenes'][0]['name']==descriptor['model_scene']=='default'
    link(native,folder/'model.glb');link(delivery,folder/'lossy.glb');receipt=dict(asset_id=ref['id'],source=sha(native),output=sha(delivery),settings=dict(geometry='lossless',image_reencoding=False),authority_model_sha256=e['source_sha256'],user_approval_sha256=sha(approval),root_review_sha256=sha(work/'root-review-v4.json'),scene_name_guard_sha256=sha(work/'scene-name-guard.json'),live_crown_guard_sha256=sha(work/'live-crown-sampler-guard-v1.json'),geometry_uv_all_accessors_exact=True,materials_nodes_samplers_exact=True);write(folder/'lossy.glb.receipt.json',receipt)
    for name in ['model.glb','lossy.glb','lossy.glb.receipt.json']:
     source=folder/name;target=src.parent/name;check_gameplay_preserved(source,target);mapping[str(target.relative_to(L/'3d-assets'))]=source;records.append(dict(source=str(source),target=str(target),source_sha256=sha(source),previous_sha256=sha(target),backup=str(S/'promotion-backup'/f'{len(records):04d}-{target.name}')))
    assert not entry.get('preview_model'),'Existing preview needs explicit replacement, never inherit stale preview'
    deltas.append(dict(asset_id=ref['id'],source_sha256=sha(native),delivery_sha256=sha(delivery),descriptor_sha256=sha(src),pivot=e['pivot'],parts=e['parts'],authority_files={str(work/f):sha(work/f)for f in ['root-review-v4.json','self-review-v4.json','source-surface-guard.json','delivery-proof.json','scene-name-guard.json','live-crown-sampler-guard-v1.json','export.json']}))
   assert sha(folder/'asset.json')==sha(src);pins.append(dict(asset_id=ref['id'],descriptor_sha256=sha(src),changed=changed,model_sha256=sha(folder/'model.glb'),delivery_sha256=sha(folder/'lossy.glb')if(folder/'lossy.glb').exists()else None))
  candidate=copy.deepcopy(baseline);pins_byid={x['asset_id']:x for x in pins}
  for ref in candidate['assetSources']+candidate['sceneAssets']:
   if ref['id']in target_ids:ref['model_sha256']=pins_byid[ref['id']]['model_sha256']
  restored=copy.deepcopy(candidate)
  for ref,old in zip(restored['assetSources']+restored['sceneAssets'],baseline['assetSources']+baseline['sceneAssets']):ref['model_sha256']=old['model_sha256']
  assert restored==baseline;mapfile=S/'croisement02.rhlos-map.json';write(mapfile,candidate);assert candidate['placements']==baseline['placements']
  staged_index=discover_asset_index(root);inherited_problems=verify_derivatives(root);assert inherited_problems==verify_derivatives(L/'3d-assets',index={'assets':[byid[r['id']]for r in refs]}),inherited_problems;assert not verify_derivatives(root,index={'assets':[e for e in staged_index['assets']if e['id']in target_ids]});write_asset_index(root);merged=S/'promotion-library-index.json';write_asset_index(L/'3d-assets',target=merged,files=mapping);after=read(merged);generated_byid={x['id']:x for x in after['assets']};assert set(generated_byid)==set(byid);scoped_index=copy.deepcopy(index);scoped_index['assets']=[generated_byid[x['id']] if x['id'] in target_ids else x for x in scoped_index['assets']];write(merged,scoped_index);new_byid={x['id']:x for x in scoped_index['assets']};assert all(new_byid[x]==byid[x]for x in byid if x not in target_ids)
  records.extend([dict(source=str(mapfile),target=str(live_map),source_sha256=sha(mapfile),previous_sha256=EXPECTED_MAP,backup=str(S/'promotion-backup/map.json')),dict(source=str(merged),target=str(L/'3d-assets/index.json'),source_sha256=sha(merged),previous_sha256=index_sha,backup=str(S/'promotion-backup/index.json'))])
  protected=[L/'mission-states/index.json',L/'scenes/croisement02-volumes.scene.json',L/'scenes/croisement02-volumes.scene.glb'];protected +=[L/f'scenes/{name}'for name in os.listdir(L/'scenes')if name.endswith('.rhlos-map.json')and name!='croisement02.rhlos-map.json']
  manifest=dict(status='PENDING_BROWSER_NOT_APPLIED',inherited_derivative_holds=inherited_problems,stage=str(S),library=str(L),files=records,protected_files=[dict(path=str(p),sha256=sha(p))for p in protected],index_generation=dict(target=str(L/'3d-assets/index.json')),browser_check=dict(status='PENDING',scope='Exact individual full/close derivatives root-PASS; combined staged/library verification remains required.'),publication_scope='Only five approved tree model/delivery/receipt replacements plus model pins; every gameplay descriptor and placement unchanged.',approval_sha256=sha(approval));write(S/'promotion-draft.json',manifest)
  proof=dict(status='PASS scoped five-model candidate; browser and inherited derivative holds pending',inherited_derivative_holds=inherited_problems,baseline_map_sha256=EXPECTED_MAP,map_sha256=sha(mapfile),unchanged_descriptor_count=len(pins),placements_exact=True,gameplay_descriptors_exact=True,other_map_index_entries_exact=True,other_asset_model_pins_exact=True,only_five_source_model_pins_changed=True,deltas=deltas,assets=pins,publication=False);write(S/'candidate.json',proof);write(S/'scope.json',dict(asset_ids=sorted(target_ids),already_published=sorted(set(pins_byid)-target_ids),required_patches=[]))
  assert sha(live_map)==EXPECTED_MAP and sha(L/'3d-assets/index.json')==index_sha
 print(S,sha(S/'candidate.json'),flush=True)
if __name__=='__main__':main()
