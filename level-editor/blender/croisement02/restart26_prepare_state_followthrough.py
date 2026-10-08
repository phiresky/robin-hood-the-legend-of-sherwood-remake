"""Validate private scatter transport and prepare deferred browser/bake requests."""
import hashlib,io,json,shlex,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement'
PARENT=BASE/'restart25-approved-state-materialization-v1';DEST=BASE/'restart26-state-followthrough-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return {'path':str(p),'sha256':sha(p)}
def read(p):return json.loads(Path(p).read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def glb(p):
 b=Path(p).read_bytes();assert struct.unpack_from('<III',b)==(0x46546c67,2,len(b));size,kind=struct.unpack_from('<II',b,12);assert kind==0x4e4f534a;return json.loads(b[20:20+size]),b[20+size:]
def main():
 DEST.mkdir(exist_ok=False)
 source=PARENT/'scatter-export-v2/report.json';report=read(source);model=Path(report['model']);assert sha(model)==report['model_sha256']=='8dde9e757a7333f3a8862e4228d49eb2ab0ca50eaf04fc2183574e6c0c8d717c'
 assert sha(report['approval']['member']['model'])==report['approval']['member']['model_sha256'];assert sha(report['approval']['path'])==report['approval']['sha256']
 before,binary_before=glb(PARENT/'scatter-export-v1/scatter.glb');doc,binary=glb(model);assert binary==binary_before
 for k in ['accessors','bufferViews','buffers','meshes','nodes','scenes','images','textures','samplers']:assert doc[k]==before[k],k
 assert not doc.get('animations') and 'KHR_materials_unlit' in doc['extensionsRequired'];assert len(doc['scenes'])==20
 materials=[]
 for i,mat in enumerate(doc['materials']):
  old=before['materials'][i];assert mat['name']==old['name'];assert mat['alphaMode']=='BLEND' and mat['doubleSided'] and mat['extensions']['KHR_materials_unlit']=={}
  pbr=mat['pbrMetallicRoughness'];assert pbr['baseColorFactor']==[1,1,1,1] and pbr['metallicFactor']==0 and pbr['roughnessFactor']==1
  texture=pbr['baseColorTexture']['index'];assert texture==old['emissiveTexture']['index'];tex=doc['textures'][texture];sampler=doc['samplers'][tex['sampler']]
  assert sampler['magFilter']==9728 and sampler['minFilter']==9984 and sampler.get('wrapS',10497)==sampler.get('wrapT',10497)==10497
  image=doc['images'][tex['source']];view=doc['bufferViews'][image['bufferView']];start=view.get('byteOffset',0);rgba=np.array(Image.open(io.BytesIO(binary[8+start:8+start+view['byteLength']])).convert('RGBA'))
  assert (rgba[:,:,3]==0).any() and (rgba[:,:,3]>0).any()
  materials.append({'name':mat['name'],'alpha_zero_pixels':int((rgba[:,:,3]==0).sum()),'positive_alpha_pixels':int((rgba[:,:,3]>0).sum()),'rgba_sha256':hashlib.sha256(rgba.tobytes()).hexdigest(),'semantics':'unlit RGBA blend; double-sided; nearest magnification, nearest mipmap minification; repeat wrap'})
 def positions(prim):
  a=doc['accessors'][prim['attributes']['POSITION']];v=doc['bufferViews'][a['bufferView']];assert a['componentType']==5126 and a['type']=='VEC3' and not v.get('byteStride');return np.frombuffer(binary[8:],dtype='<f4',count=a['count']*3,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(-1,3)
 bindings=read(PARENT/'approved-bindings.json');instances={r['id']:r for r in bindings['bindings']};records=[];seen=set()
 for scene,expected in zip(doc['scenes'],report['scenes']):
  assert scene['name']==expected['name'];group=doc['nodes'][scene['nodes'][0]];assert not any(k in group for k in ['matrix','translation','rotation','scale']);children=[doc['nodes'][i] for i in group['children']];assert sorted(n['name'] for n in children)==sorted(expected['objects'])
  points=[]
  for n in children:
   assert not any(k in n for k in ['matrix','translation','rotation','scale']);assert n['name'] not in seen;seen.add(n['name']);points.extend(positions(p) for p in doc['meshes'][n['mesh']]['primitives'])
  p=np.concatenate(points);assert np.isfinite(p).all();aliases=[]
  for id in expected['instances']:
   r=instances[id];binding=r['endpoints']['applied']['private_runtime_candidate'];assert binding['model_scene']==scene['name'] and binding['sha256']==sha(model)
   assert r['endpoints']['applied']['placement']=={'kind':'saved-world-transforms','translation':[0,0,0]}
   aliases.append({'id':id,'mission':r['mission'],'terminal_tick':r['terminal_tick'],'duration':r['transition_duration'],'source_contract':r['source_contract'],'initial_kind':r['endpoints']['initial']['kind'],'initial_asset_ready':r['endpoints']['initial']['kind']=='source-absent','applied_position':[0,0,0]})
  records.append({'scene':scene['name'],'object_names':expected['objects'],'gltf_y_up_bounds':[p.min(0).tolist(),p.max(0).tolist()],'aliases':aliases})
 assert len(seen)==23 and sum(len(r['aliases']) for r in records)==32
 runtime=[ROOT/'level-editor/app/src'/f for f in ['state-delivery.ts','scene-assets.ts','editor-viewport.ts','native-state-presentation.ts']]+[ROOT/'level-editor/shared/src/state-delivery.ts']
 request={'status':'CPU_VALIDATED_DEFERRED_BROWSER_REQUEST','model':pin(model),'source_worker':pin(Path(report['approval']['member']['model'])),'source_report':pin(source),'materials':materials,'scenes':records,'source_triangle_uv_evidence':report['source_triangle_uv_checks'],'binary_geometry_images_unchanged':True,'production_path':['SceneAssetLoader.load','StateDelivery.set','EditorViewport.setStateDelivery'],'prohibited_path':'MissionStateLayer.set requires animated target clips and is not the static endpoint path','runtime_snapshot':[pin(p) for p in runtime],'required_repin':'Browser owner must rehash current runtime after ongoing appearance publication; no silent stale-pin waiver','reader':'Read-only HTTP directory handle rooted at private candidate bundle; do not insert fake library index entries','canonical_authority':bindings['source_catalog'],'preconditions':['Appearance publication/browser slot complete','Exact current receiver context and dependencies frozen','For31 hiding controls, approved textured initial mound bindings must exist before full production delivery contract; do not substitute absent. One orphan has proven source-absent initial.'],'checks':['All20 scenes load only their declared meshes; match world bounds withzero extra translation','All23 materials are transparent unlit sourceRGBA; positive and zero alpha samples match CPU oracle','Capture source and oblique appearance for20 unique scenes with actual current terrain receivers; verify noopaque rectangle, double placement, zfighting or unwanted receiver replacement','Forall32 aliases: initial -> forward terminal-1 -> terminal -> applied -> forcedreset; physical endpoint appears onlyatlegitimate terminal, nativeart handles intermediate frames','Switch missions twice and Map-only; no stale scatter roots/background stamping, resource leaks or duplicatealiases','Capture exact source contract resource hashes, model receipt, and pre/post current catalog/map/runtime hashes; report meaningful errors rather than retrying readiness blindly'],'scope':'Private request only. Source-backed final scatter appearance isapproved; current receiver/context and production browser behavior stillunverified. No continuous leaf motion inferred.'}
 write(DEST/'scatter-browser-request.json',request)
 batch=read(PARENT/'official-texture-experiments-v2/batch.json');commands=['#!/bin/sh','set -eu','cd '+shlex.quote(str(ROOT))];bakes=[]
 for r in batch['packets']:
  exp=Path(r['experiment']);output=exp/'bake-v1-native-restoration-pending';review=exp/'generation-review.json'
  for file,key in [('input.png','input_sha256'),('mask.png','mask_sha256'),('solid.png','solid_sha256'),('approval.json','approval_sha256'),('views.json','views_sha256')]:assert sha(exp/file)==r[key]
  command=['/usr/bin/blender','--background','--threads','2','--python-exit-code','1','--python','level-editor/blender/croisement02/bake_texture_candidate.py','--',str(exp),'--output',str(output),'--review',str(review),'--texels-per-unit','2','--view-selection','best-facing-single']
  commands.extend(['test -f '+shlex.quote(str(review)),shlex.join(command)])
  bakes.append({'asset_id':r['asset_id'],'experiment':str(exp),'output':str(output),'review_path':str(review),'command':command,'ready_to_run':False,'required_review_fields':['reviewer','ready_for_bake=true','asset_id','raw_image','raw_sha256','preserved_image','preserved_sha256','input_sha256','mask_sha256','model_sha256'],'preconditions':['Generatedraw+preserved images exist, manually inspected,exact approvedsize andprotectedRGBA+alpha pass','Generationreview is factual,hashbound and readyforbake; no placeholderapproval','Root grants renderlane;>=6GiBmemory and>=10GiBdisk'],'postconditions':['Reopen geometry/UV/source material guards','Restore exact native shader/knowntexels before actualmodel review; genericbakealone isnot finalnativepreservation proof','Moundreuse onlyunknown surfaceappearance:67templateclumps,20rigidplacements, all20ownedtexeldomainsrestore independently','Eightactualviews andsourcecontext review; finalappearance decisionpending']})
 write(DEST/'bake-plan.json',{'status':'DETERMINISTIC_COMMANDS_BLOCKED_ON_UNGENERATED_OUTPUTS_AND_REVIEW','shared_baker':pin(ROOT/'level-editor/blender/croisement02/bake_texture_candidate.py'),'experiments':bakes,'generation_launched':False,'bake_launched':False})
 (DEST/'bake-reviewed-manually.sh').write_text('\n'.join(commands)+'\n');print(json.dumps({'browser_request':str(DEST/'scatter-browser-request.json'),'bakes':len(bakes),'scenes':len(records),'aliases':32}))
if __name__=='__main__':main()
