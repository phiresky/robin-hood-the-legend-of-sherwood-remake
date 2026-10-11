"""Append private source-plane crown morphs while preserving static GLB bytes."""
import argparse
import copy
import hashlib
import json
import math
import struct
import shutil
from pathlib import Path
import numpy as np
from scipy.ndimage import map_coordinates
from scipy.spatial.transform import Rotation
from forest_layout import CROWNS
from restart26_tree42_approved_morph_export import phase_weights

ROOT=Path(__file__).resolve().parents[3]
LIB=ROOT/'level-editor/library'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--fields',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--masks',type=int,nargs='+')
    args=parser.parse_args();args.output.mkdir(exist_ok=False)
    report=json.loads((args.fields/'report.json').read_text())
    packet=np.load(args.fields/'flows.npz');flows=packet['flow'];left,top,w,h=map(int,packet['bbox'])
    assert flows.shape==(14,h,w,2) and np.all(flows[0]==0)
    map_path=LIB/'scenes/croisement02.rhlos-map.json';map_bytes=map_path.read_bytes();scene=json.loads(map_bytes)
    records=[]
    if args.masks:assert set(args.masks)<=set(x[0] for x in CROWNS[report['animation_index']])
    for mask,_,_ in CROWNS[report['animation_index']]:
        if args.masks and mask not in args.masks:continue
        assert shutil.disk_usage(args.output).free>=10*1024**3
        asset=f'croisement02-tree-{mask:02}';model=LIB/f'3d-assets/croisement02/{asset}/model.glb'
        source_bytes=model.read_bytes();jlen=struct.unpack_from('<I',source_bytes,12)[0];doc=json.loads(source_bytes[20:20+jlen]);start=28+jlen
        assert not doc.get('animations') and not doc.get('skins')
        original=copy.deepcopy(doc)
        buffers=[source_bytes[start:start+doc['buffers'][0]['byteLength']]]
        dependencies=[]
        for buf in doc['buffers'][1:]:
            p=(model.parent/buf['uri']).resolve();buffers.append(p.read_bytes());dependencies.append(dict(path=str(p),sha256=sha(p)))
        data=bytearray(buffers[0])
        for bi,buf in enumerate(buffers[1:],1):
            data.extend(b'\0'*(-len(data)%4));offset=len(data);data.extend(buf)
            for v in doc['bufferViews']:
                if v.get('buffer',0)==bi:v['buffer']=0;v['byteOffset']=v.get('byteOffset',0)+offset
        doc['buffers']=[dict(byteLength=len(data))]
        def read_accessor(index):
            a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']]
            assert a['componentType']==5126 and a['type']=='VEC3' and 'sparse' not in a
            return np.ndarray((a['count'],3),dtype='<f4',buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',12),4)).copy()
        def append(array,kind):
            a=np.asarray(array,dtype='<f4');data.extend(b'\0'*(-len(data)%4));offset=len(data);data.extend(a.tobytes());vi=len(doc['bufferViews']);doc['bufferViews'].append(dict(buffer=0,byteOffset=offset,byteLength=a.nbytes));width=3 if kind=='VEC3' else 1;values=a.reshape(-1,width);ai=len(doc['accessors']);doc['accessors'].append(dict(bufferView=vi,componentType=5126,count=len(values),type=kind,min=values.min(0).tolist(),max=values.max(0).tolist()));return ai
        placement=next(p for p in scene['placements'] if p['id']==asset)
        transform=placement['transform'];assert transform['rot_deg']==0
        placed=np.eye(4);placed[:3,3]=[transform['dx'],transform['dz'],transform['dy']/S]
        crowns=[]
        def visit(index,parent):
            node=doc['nodes'][index]
            if 'matrix' in node:local=np.array(node['matrix']).reshape(4,4).T
            else:
                local=np.eye(4);local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
            world=parent@local
            if node.get('extras',{}).get('projection_component')=='crown':crowns.append((index,world))
            for child in node.get('children',[]):visit(child,world)
        for index in doc['scenes'][doc.get('scene',0)]['nodes']:visit(index,placed)
        assert len(crowns)==1
        index,world=crowns[0];mesh=doc['meshes'][doc['nodes'][index]['mesh']];stats=[]
        for primitive in mesh['primitives']:
            assert not primitive.get('targets')
            positions=read_accessor(primitive['attributes']['POSITION']);points=positions@world[:3,:3].T+world[:3,3]
            xy=np.c_[points[:,0],points[:,2]*S-points[:,1]*C]
            deltas=[]
            for phase in range(1,14):
                du=map_coordinates(flows[phase,:,:,0],[xy[:,1]-top,xy[:,0]-left],order=1,mode='constant',cval=0)
                dv=map_coordinates(flows[phase,:,:,1],[xy[:,1]-top,xy[:,0]-left],order=1,mode='constant',cval=0)
                world_delta=np.c_[du,-dv*C,dv*S]
                assert np.max(np.abs(world_delta@np.array([0,S,C])))<1e-6
                deltas.append(world_delta@np.linalg.inv(world[:3,:3]).T)
            primitive['targets']=[dict(POSITION=append(delta,'VEC3')) for delta in deltas]
            stats.append(dict(vertices=len(positions),maximum_local_motion=float(np.max(np.linalg.norm(deltas,axis=2)))))
        mesh['weights']=[0]*13;mesh.setdefault('extras',{})['targetNames']=[f'Native phase {i:02}' for i in range(1,14)]
        times,weights=phase_weights();doc['animations']=[dict(name=f'Private canopy {report["animation_index"]:02} motion proposal',samplers=[dict(input=append(times,'SCALAR'),output=append(weights,'SCALAR'),interpolation='STEP')],channels=[dict(sampler=0,target=dict(node=index,path='weights'))])]
        assert original['materials']==doc['materials'] and original['images']==doc['images']
        assert bytes(data[:len(buffers[0])])==buffers[0]
        doc['buffers'][0]['byteLength']=len(data);data.extend(b'\0'*(-len(data)%4));encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4)
        output=args.output/f'{asset}.glb';output.write_bytes(struct.pack('<III',0x46546c67,2,28+len(encoded)+len(data))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(data),0x004e4942)+data)
        assert sha(model)==hashlib.sha256(source_bytes).hexdigest()
        records.append(dict(asset_id=asset,placement=placement,source_model=dict(path=str(model),sha256=sha(model)),dependencies=dependencies,model=dict(path=str(output),sha256=sha(output)),primitives=stats,crown_node=index,static_binary_prefix_exact=True,materials_images_unchanged=True,wood_has_no_animation=True))
        print(asset,sha(output),flush=True)
    result=dict(status='PRIVATE_PHYSICAL_MORPH_PROPOSALS_PENDING_SAVED_MODEL_REVIEW',animation_index=report['animation_index'],source_map_sha256=hashlib.sha256(map_bytes).hexdigest(),fields_sha256=sha(args.fields/'flows.npz'),field_report_sha256=sha(args.fields/'report.json'),recipe_sha256=sha(Path(__file__)),assets=records,cycle_ticks=56,pose_count=14,closing_pose=0,limits=['New motion is unapproved. Static approved geometry/material bytes are retained, but derivatives require independent importer and visual/contact review.','Image-plane motion extends through crown depth coherently; hidden deformation and crown partitions remain inferred.','Static normals retained on source-colored materials; no physical relighting claim.','No mission catalog, canonical or runtime edits.'])
    (args.output/'export.json').write_text(json.dumps(result,indent=2)+'\n')


if __name__=='__main__':main()
