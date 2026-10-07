"""Translate the explicitly approved receiver mask into an exact planar API packet."""
import io,json,hashlib,struct,sys
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'))
from prepare_planar_texture_packet import coverage
B=R/'level-editor/work/croisement03-refinement/restart2';A=B/'approved-hub-v17-v23-plus-two-v1';D=B/'trio-tree-integration-v1/terrain-input-proposal-v5';O=B/'approved-hub-textures-v1/trio-ground/experiment'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 scope=json.loads((A/'verified-scope.json').read_text());member=scope['effective_assets']['croisement03-trio-ground-receiver'];assert member['scope']=='texture-input';assert sha(member['model'])==member['model_sha256'];assert sha(member['source_evidence'])==member['source_evidence_sha256'];bound=json.loads(Path(member['source_evidence']).read_text())['items'][0]
 for p,h in bound['evidence'].items():assert sha(p)==h,p
 for key in ['croisement03-tree-12','croisement03-tree-14']:assert sha(scope['effective_assets'][key]['model'])==scope['effective_assets'][key]['model_sha256']
 model=Path(member['model']);raw=model.read_bytes();n=struct.unpack_from('<I',raw,12)[0];g=json.loads(raw[20:20+n]);chunk=raw[28+n:];buffers={0:chunk};deps={}
 for i,buffer in enumerate(g['buffers']):
  if 'uri' in buffer:
   p=(model.parent/buffer['uri']).resolve();content=p.read_bytes();digest=hashlib.sha256(content).hexdigest();assert p.stem==digest,'Content-addressed geometry buffer changed';buffers[i]=content;deps[str(p)]=digest
 assert len(g['meshes'])==1 and len(g['meshes'][0]['primitives'])==1
 primitive=g['meshes'][0]['primitives'][0];material=g['materials'][primitive['material']];assert material.get('alphaMode','OPAQUE')=='OPAQUE' and 'KHR_materials_unlit' in material['extensions']
 def accessor(index):
  a=g['accessors'][index];v=g['bufferViews'][a['bufferView']];dtype={5126:'<f4',5123:'<u2',5125:'<u4'}[a['componentType']];cols={'SCALAR':1,'VEC2':2,'VEC3':3}[a['type']];return np.ndarray((a['count'],cols),dtype=dtype,buffer=buffers[v.get('buffer',0)],offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',np.dtype(dtype).itemsize*cols),np.dtype(dtype).itemsize)).copy()
 positions=accessor(primitive['attributes']['POSITION']);assert np.ptp(positions[:,2])<1e-7;uv=accessor(primitive['attributes']['TEXCOORD_0']);indices=accessor(primitive['indices']).ravel().reshape(-1,3);physical=coverage(uv[indices],1408,960)
 iv=g['bufferViews'][g['images'][0]['bufferView']];embedded=Image.open(io.BytesIO(buffers[iv.get('buffer',0)][iv.get('byteOffset',0):iv.get('byteOffset',0)+iv['byteLength']])).convert('RGB');source=Image.open(D/'decoded-ground-original.png').convert('RGB');assert np.array_equal(np.array(embedded),np.array(source))
 domain=np.array(Image.open(D/'candidate-unknown-domain.png'))>0;assert int(domain.sum())==17447 and np.all(physical[domain]);O.mkdir(parents=True,exist_ok=False);image=np.array(source.convert('RGBA'));image[domain,:3]=128;Image.fromarray(image).save(O/'input.png');mask=np.full_like(image,255);mask[domain,3]=0;Image.fromarray(mask).save(O/'mask.png');solid=np.zeros_like(image);solid[physical,:3]=128;solid[physical,3]=255;Image.fromarray(solid).save(O/'solid.png')
 assert np.array_equal(image[~domain,:3],np.array(source)[~domain]);assert source.size==(1408,960)
 manifest=dict(asset_id=member['asset_id'],projection_kind='planar-atlas',layout=dict(width=1408,height=960),views=[dict(index=0,input='input.png',mask='mask.png',crop=dict(left=0,top=0,width=1408,height=960))],source_image=str(D/'decoded-ground-original.png'),atlas_source=str(D/'decoded-ground-original.png'),atlas_sha256=sha(D/'decoded-ground-original.png'),audited_glb=str(model),audited_glb_sha256=sha(model),external_geometry_buffers=deps,physical_coverage='Exact two-triangle unlit planar terrain atlas; approved editable pixels all inside physical UV coverage.',geometry_revision=member['review_revision'],ownership_report=str(D/'scope.json'),ownership_sha256=sha(D/'scope.json'),approved_receiver_mask=str(D/'candidate-unknown-domain.png'),approved_receiver_mask_sha256=sha(D/'candidate-unknown-domain.png'),approval_receipt=str(A/'user-approval.json'),approval_receipt_sha256=sha(A/'user-approval.json'),translation='Exact approved1408x960 source/mask, neutralize only17447 editable pixels. No reframe, resize, new crop or geometry changes. Direct texture-input approval replaces an eight-view geometry packet for this planar receiver.')
 write(O/'views.json',manifest);write(O/'approval.json',dict(status='approved',approved_by='user',asset_id=member['asset_id'],input_sha256=sha(O/'input.png'),solid_sha256=sha(O/'solid.png'),mask_sha256=sha(O/'mask.png'),geometry_revision=member['review_revision'],exact_user_text=scope['exact_user_text'],source_decision=member,source_receipt_sha256=scope['receipt_sha256'],scope='Exact approved receiver input and compatibleTree12/14 geometry. Generated appearance pending.'))
 refs=[]
 for p in sorted(D.glob('reference-*.png')):refs.append(dict(source='material',file=str(p),sha256=sha(p),asset_id='croisement03-observed-ground',role='Approved same-map observed moss and fallen leaf floor; material only. Do not add roots, trunks, standing plants or scenery silhouettes.'))
 assert len(refs)==2;write(O/'auxiliary-references.json',dict(input_sha256=sha(O/'input.png'),lighting_sha256=sha(O/'solid.png'),references=refs));write(O/'preparation.json',dict(status='Exact approved planar receiver prepared; visual packet check before API',editable_pixels=int(domain.sum()),protected_pixels=int((~domain).sum()),physical_pixels=int(physical.sum()),geometry_vertices=len(positions),geometry_triangles=len(indices),external_geometry_buffers=deps,files={p.name:sha(p) for p in O.iterdir() if p.is_file()}));print(O)
if __name__=='__main__':main()
