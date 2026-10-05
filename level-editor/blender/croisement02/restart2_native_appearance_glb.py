"""Export exact native appearance phases as an explicitly planar, timed GLB proof."""
import json,struct,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def build(assembly,source=None,dest=None,loop=False):
    source=source or OUT/'state-target-evidence'/assembly/'full-motion'
    manifest=json.loads((source/'manifest.json').read_text())
    dest=dest or OUT/'restart2-state'/f'{assembly}-native-appearance-v1';dest.mkdir(parents=True,exist_ok=False)
    rows=manifest['records'];w,h=Image.open(source/rows[0]['image']).size
    binary=bytearray();d=dict(asset=dict(version='2.0',generator='Native appearance proof; no recovered 3D motion'),buffers=[dict(byteLength=0)],bufferViews=[],accessors=[],images=[],textures=[],samplers=[dict(magFilter=9728,minFilter=9728,wrapS=33071,wrapT=33071)],materials=[],meshes=[],nodes=[],scenes=[dict(nodes=[])],scene=0,extensionsUsed=['KHR_materials_unlit'],animations=[dict(name='Native appearance transition',samplers=[],channels=[])])
    def view(data):
        binary.extend(b'\0'*((-len(binary))%4));start=len(binary);binary.extend(data);index=len(d['bufferViews']);d['bufferViews'].append(dict(buffer=0,byteOffset=start,byteLength=len(data)));return index
    def accessor(values,kind,component=5126):
        a=np.asarray(values,dtype='<f4'if component==5126 else '<u2');index=len(d['accessors']);record=dict(bufferView=view(a.tobytes()),componentType=component,count=len(a),type=kind)
        if kind=='VEC3':record.update(min=a.min(axis=0).tolist(),max=a.max(axis=0).tolist())
        elif kind=='SCALAR':record.update(min=[float(a.min())],max=[float(a.max())])
        d['accessors'].append(record);return index
    positions=accessor([[-w/2,h/2,0],[w/2,h/2,0],[-w/2,-h/2,0],[w/2,-h/2,0]],'VEC3')
    uv=accessor([[0,0],[1,0],[0,1],[1,1]],'VEC2');indices=accessor([0,2,1,1,2,3],'SCALAR',5123)
    times=[r['first_tick']/25 for r in rows]+[(manifest['terminal_tick']+1)/25];time_accessor=accessor(times,'SCALAR');bindings=[]
    for index,row in enumerate(rows):
        path=source/row['image'];rgba=np.asarray(Image.open(path).convert('RGBA'));assert hashlib.sha256(rgba.tobytes()).hexdigest()==row['rgba_sha256']
        d['images'].append(dict(bufferView=view(path.read_bytes()),mimeType='image/png'));d['textures'].append(dict(sampler=0,source=index))
        d['materials'].append(dict(name=f'Exact native appearance tick {row["first_tick"]}',pbrMetallicRoughness=dict(baseColorTexture=dict(index=index),metallicFactor=0,roughnessFactor=1),extensions={'KHR_materials_unlit':{}},alphaMode='MASK',alphaCutoff=.5,doubleSided=True))
        d['meshes'].append(dict(primitives=[dict(attributes=dict(POSITION=positions,TEXCOORD_0=uv),indices=indices,material=index)]))
        d['nodes'].append(dict(name=f'Native phase {index:03}',mesh=index,scale=[1,1,1]if index==0 else [0,0,0],extras=dict(native_first_tick=row['first_tick'],representation='planar native appearance; not moving solid geometry')));d['scenes'][0]['nodes'].append(index)
        scales=[[1,1,1]if phase==index else [0,0,0]for phase in range(len(rows))]+[[1,1,1]if index==(0 if loop else len(rows)-1) else [0,0,0]]
        d['animations'][0]['samplers'].append(dict(input=time_accessor,output=accessor(scales,'VEC3'),interpolation='STEP'));d['animations'][0]['channels'].append(dict(sampler=index,target=dict(node=index,path='scale')))
        bindings.append(dict(image=row['image'],png_sha256=sha(path),rgba_sha256=row['rgba_sha256'],first_tick=row['first_tick'],last_tick=row['last_tick']))
    binary.extend(b'\0'*((-len(binary))%4));d['buffers'][0]['byteLength']=len(binary);j=json.dumps(d,separators=(',',':')).encode();j+=b' '*((-len(j))%4);body=struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(binary),0x004e4942)+binary;path=dest/'native-appearance.glb';path.write_bytes(struct.pack('<III',0x46546c67,2,len(body)+12)+body)
    report=dict(status='Exported private planar appearance proof; browser verification pending',assembly=assembly,loop=loop,source_directory=str(source),glb_sha256=sha(path),source_manifest_sha256=sha(source/'manifest.json'),width=w,height=h,tick_rate=25,terminal_tick=manifest['terminal_tick'],channels=len(rows),times=times,source_bindings=bindings,limitations=['This is a planar native appearance animation, not recovered rigid-body 3D motion.','No fabricated intermediate body identity or physics is claimed.','Geometry endpoints, shadow receivers, metadata swap, sounds and mission activation require separate integration.','Loop at the exact native cycle boundary.' if loop else 'The last phase must clamp; this transition is not a looping idle animation.'])
    (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(assembly,len(rows),sha(path))


if __name__=='__main__':
    for assembly in ['log-trap','rock-trap']:build(assembly)
