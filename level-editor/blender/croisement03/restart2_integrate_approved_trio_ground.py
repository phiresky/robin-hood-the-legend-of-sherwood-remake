"""Privately embed the approved ground atlas without geometry or gameplay edits."""
import copy,hashlib,io,json,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def glb(path):
 raw=Path(path).read_bytes();n=struct.unpack_from('<I',raw,12)[0];d=json.loads(raw[20:20+n]);binary=raw[28+n:];buffers=[(Path(path).parent/x['uri']).resolve().read_bytes() if 'uri'in x else binary[:x['byteLength']] for x in d['buffers']];return d,buffers

def main():
 out=B/'approved-trio-ground-integration-v1';assert not out.exists();receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/next-ground-storehouse-hole-v1/user-approval-ground.json';approval=read(receipt);assert approval['status']=='USER_APPROVED';decision=approval['decision'];src=Path(decision['model']);atlas=Path(decision['atlas']);assert sha(src)==decision['model_sha256']=='ab505cb2bcc3d4b27c4179518b489a7e69a199299c96fb6e34ce1743fbff1a82';assert sha(atlas)==decision['atlas_sha256']=='694ac0d631c047f0ed7f854dc82ef5bb2f87505132148b6783dbee68b0f7a268'
 exp=atlas.parent.parent;view=read(exp/'views.json');mask=rgba(exp/'mask.png')[:,:,3]==0;filled=rgba(atlas);original=rgba(view['atlas_source']);assert mask.sum()==17447 and filled.shape==original.shape==(960,1408,4);assert np.array_equal(filled[~mask],original[~mask]) and np.array_equal(filled[:,:,3],original[:,:,3])
 doc,buffers=glb(src);before=copy.deepcopy(doc);assert len(doc['images'])==len(doc['meshes'])==1;imageview=doc['images'][0]['bufferView'];v=doc['bufferViews'][imageview];oldbytes=buffers[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']];assert np.array_equal(rgba(io.BytesIO(oldbytes)),original)
 newblob=bytearray();viewproof=[]
 for i,v in enumerate(doc['bufferViews']):
  source=before['bufferViews'][i];old=buffers[source.get('buffer',0)][source.get('byteOffset',0):source.get('byteOffset',0)+source['byteLength']];payload=atlas.read_bytes() if i==imageview else old;newblob.extend(b'\0'*(-len(newblob)%4));v.update(buffer=0,byteOffset=len(newblob),byteLength=len(payload));newblob.extend(payload);viewproof.append(dict(index=i,image=i==imageview,original_sha256=hashlib.sha256(old).hexdigest(),output_sha256=hashlib.sha256(payload).hexdigest(),geometry_exact=i!=imageview and payload==old))
 doc['buffers']=[dict(byteLength=len(newblob))];doc['images'][0]['mimeType']='image/png';probe=copy.deepcopy(doc);probe['buffers']=before['buffers'];probe['bufferViews']=before['bufferViews'];probe['images']=before['images'];assert probe==before
 out.mkdir();j=json.dumps(doc,separators=(',',':')).encode();j+=b' '*(-len(j)%4);binary=bytes(newblob)+b'\0'*(-len(newblob)%4);model=out/'model.glb';model.write_bytes(struct.pack('<III',0x46546c67,2,28+len(j)+len(binary))+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(binary),0x004e4942)+binary)
 rd,rb=glb(model);assert rd==doc;v=rd['bufferViews'][imageview];stored=rb[0][v['byteOffset']:v['byteOffset']+v['byteLength']];assert stored==atlas.read_bytes() and np.array_equal(rgba(io.BytesIO(stored)),filled)
 for i,prior in enumerate(before['bufferViews']):
  if i==imageview:continue
  v=rd['bufferViews'][i];assert rb[0][v['byteOffset']:v['byteOffset']+v['byteLength']]==buffers[prior.get('buffer',0)][prior.get('byteOffset',0):prior.get('byteOffset',0)+prior['byteLength']]
 descriptor=read(src.parent/'asset.json');candidate=copy.deepcopy(descriptor);candidate['resources']=[];assert candidate['gameplay']==descriptor['gameplay'];write(out/'asset.json',candidate)
 rows=[]
 for tree in [12,14]:
  p=B/f'trio-tree-integration-v1/static-leaf-source-proposal-v2/tree{tree}-proposed-source-rgba.png';owned=rgba(p)[:,:,3]>0;rows.append(dict(tree=tree,kind='approved static-native foliage',pixels=int(owned.sum()),filled=int((owned&mask).sum()),protected=int((owned&~mask).sum()),policy='Keep approved static-native leaf surfaces in separated tree asset. Frame0-derived crown is a distinct reserved native-animation role.'))
 for tree in [10,11,12,13,14]:
  version=2 if tree==11 else 1;p=B/f'tree{tree}-bark-proposal-v{version}/proposed-bark.png';owned=np.asarray(Image.open(p))>0;rows.append(dict(tree=tree,kind='accepted native bark',pixels=int(owned.sum()),filled=int((owned&mask).sum()),protected=int((owned&~mask).sum()),policy='Tree bark source remains on approved 3D receiver; only atlas duplicate within approved domain replaced. Tree10/11 domains are outside this scope.'))
 pins={str(p):sha(p) for p in [receipt,src,src.parent/'asset.json',atlas,exp/'mask.png',exp/'views.json',Path(view['atlas_source']),LIB/'scenes/croisement03.rhlos-map.json']}
 report=dict(status='PRIVATE exact approved atlas embedded; source and geometry guards PASS; scene ownership/runtime pending',model_sha256=sha(model),descriptor_sha256=sha(out/'asset.json'),approval_receipt_sha256=sha(receipt),atlas_sha256=sha(atlas),editable_pixels=17447,changed_pixels=int(np.any(filled!=original,axis=2).sum()),protected_changes=0,alpha_exact=True,all_geometric_buffer_views_exact=True,accessors_nodes_meshes_materials_samplers_exact=True,gameplay_exact=True,viewproof=viewproof,ownership=rows,pins=pins,limitations=['No canonical files changed. Palette/map resources, derivatives and saved reference pins remain pending.','Ground fill removes only approved pixel duplicates. Joint source and oblique scene review must preserve static-native foliage and native Arbre06 animation without simultaneous frame0 proxy.','Tree10/11 accepted bark and crowns are outside this atlas-edit scope; their separate receiver ownership remains unresolved.','Geometry/input approval and current appearance approval do not certify animation integration.'])
 write(out/'report.json',report);print(json.dumps({k:report[k] for k in ['status','model_sha256','protected_changes','ownership']},indent=2))
if __name__=='__main__':main()
