"""Restore the reviewed normal arrays after a local-pivot-only GLB export."""
import hashlib,json,struct,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/active/croisement01/restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def load(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+n]),bytearray(raw),28+n
old=R/'tree08-approved-wood-export-v2/assets/croisement01-tree-08/model.glb';source=R/'tree08-native33-export-v1';new=source/'assets/croisement01-tree-08/model.glb'
assert sha(old)=='02802ad1b5a91fadbd2e69222a64c314d89a015529a1230caaa00f8dbec5cbda';assert sha(new)=='6ee7e009c2cc7bf4febfb3cb0438d493b87c90bc13246190577261b3e6bf5838'
a,x,xb=load(old);b,y,yb=load(new)
for p,q in zip(a['meshes'][0]['primitives'],b['meshes'][0]['primitives'],strict=True):
 u=a['accessors'][p['attributes']['NORMAL']];v=b['accessors'][q['attributes']['NORMAL']]
 assert u['count']==v['count'] and u['componentType']==v['componentType']==5126 and u['type']==v['type']=='VEC3'
 av=a['bufferViews'][u['bufferView']];bv=b['bufferViews'][v['bufferView']];assert 'byteStride' not in av and 'byteStride' not in bv
 count=u['count']*12;start=xb+av.get('byteOffset',0)+u.get('byteOffset',0);target=yb+bv.get('byteOffset',0)+v.get('byteOffset',0)
 y[target:target+count]=x[start:start+count]
out=R/'tree08-native33-export-v2';shutil.copytree(source,out);dest=out/'assets/croisement01-tree-08/model.glb';dest.write_bytes(y)
proof=dict(status='NORMAL_ARRAYS_RESTORED_PENDING_EQUIVALENCE_CHECK',input_native_glb_sha256=sha(new),reviewed_glb_sha256=sha(old),output_glb_sha256=sha(dest),reason='Changing the pivot caused floating-point normal recalculation on tiny triangles. Restore reviewed normals with unchanged topology, UVs, images and materials.',source_export_proof_sha256=sha(source/'export-proof.json'))
(out/'normal-preservation.json').write_text(json.dumps(proof,indent=2)+'\n');print(proof)

export=json.loads((out/'export-proof.json').read_text());export['precursor_export_proof_sha256']=sha(source/'export-proof.json');export['files']['assets/croisement01-tree-08/model.glb']=sha(dest);export['export']['index']=str(out/'assets/index.json');export['normal_preservation_sha256']=sha(out/'normal-preservation.json');(out/'export-proof.json').write_text(json.dumps(export,indent=2)+'\n')
