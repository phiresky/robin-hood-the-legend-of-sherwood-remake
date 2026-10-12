"""Apply audited mesh ownership to a candidate worker and render grouping packets."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image

EDITOR=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(EDITOR/'refinement'))
from render_slots import acquire


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def fingerprint(objects):
    digest=hashlib.sha256()
    for obj in sorted(objects,key=lambda o:o.name):
        digest.update(obj.name.encode());digest.update(np.array(obj.matrix_world,dtype='<f4').tobytes())
        for collection,prop,size,dtype in [(obj.data.vertices,'co',3,'<f4'),(obj.data.loops,'vertex_index',1,'<i4'),
                (obj.data.polygons,'loop_total',1,'<i4'),(obj.data.polygons,'material_index',1,'<i4')]:
            values=np.empty(len(collection)*size,dtype=dtype);collection.foreach_get(prop,values);digest.update(values.tobytes())
        for layer in obj.data.uv_layers:
            uv=np.empty(len(layer.data)*2,dtype='<f4');layer.data.foreach_get('uv',uv);digest.update(layer.name.encode());digest.update(uv.tobytes())
        for material in obj.data.materials:digest.update(material.name.encode())
    return digest.hexdigest()


def world_points(objects):
    arrays=[]
    for obj in objects:
        a=np.empty(len(obj.data.vertices)*3);obj.data.vertices.foreach_get('co',a);a=a.reshape(-1,3)
        m=np.array(obj.matrix_world);arrays.append(a@m[:3,:3].T+m[:3,3])
    return np.concatenate(arrays)


def main(root, only=None):
    root=Path(root).resolve();plan=json.loads((root/'plan.json').read_text())
    assert sha(root/'catalog.json')==plan['catalog_sha256']
    source=EDITOR/'work/needs-review/sherwood-refinement/textures/reproject-v2/source-only.blend'
    assert sha(source)==plan['source_worker_sha256']
    acquire();bpy.ops.wm.open_mainfile(filepath=str(source))
    scene=bpy.data.scenes['Sherwood Editor Migration'];bpy.context.window.scene=scene
    objects=[o for o in bpy.data.collections['Sherwood Working'].objects if o.type=='MESH']
    before=fingerprint(objects)
    assert set(plan['assignments'])=={o.name for o in objects}
    split={'building-024','building-102'}
    names={g['id']:g['name'] for g in plan['groups']}
    for obj in objects:
        obj['asset_group']=plan['assignments'][obj.name]
        obj['asset_name']=names.get(obj['asset_group'],'Sherwood terrain')
        if obj['source_node'] in split:obj['projection_component']=obj.name
    assert before==fingerprint(objects)
    worker=root/'grouped-source-only.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(worker),compress=True)
    receipt=dict(status='PASS',source_worker_sha256=sha(source),worker_sha256=sha(worker),catalog_sha256=sha(root/'catalog.json'),
        mesh_count=len(objects),geometry_uv_material_fingerprint=before,geometry_uv_materials_unchanged=True,
        original_groups=plan['original_groups'],candidate_groups=plan['candidate_groups'],review_status='pending',live_library_changed=False)
    (root/'stage.json').write_text(json.dumps(receipt,indent=2)+'\n')
    # Rendering-only material copies: preserve physical foliage cutouts in both modes.
    variants={};original_slots={o.name:list(o.data.materials) for o in objects}
    for obj in objects:
        for material in original_slots[obj.name]:
            if material.name in variants:continue
            variants[material.name]={}
            for mode in ('solid','textured'):
                m=material.copy();ns,ls=m.node_tree.nodes,m.node_tree.links
                tex=next(n for n in ns if n.type=='TEX_IMAGE' and n.image)
                em=ns.new('ShaderNodeEmission')
                if mode=='textured':ls.new(tex.outputs['Color'],em.inputs['Color'])
                else:
                    geom=ns.new('ShaderNodeNewGeometry');dot=ns.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT'
                    dot.inputs[1].default_value=(-.4,-.55,.73);ls.new(geom.outputs['Normal'],dot.inputs[0])
                    mul=ns.new('ShaderNodeMath');mul.operation='MULTIPLY_ADD';mul.inputs[1].default_value=.26;mul.inputs[2].default_value=.48
                    ls.new(dot.outputs['Value'],mul.inputs[0]);ls.new(mul.outputs[0],em.inputs['Color'])
                shader=em.outputs[0]
                if material.get('foliage_physical_opacity'):
                    threshold=ns.new('ShaderNodeMath');threshold.operation='GREATER_THAN';threshold.inputs[1].default_value=.499999
                    ls.new(tex.outputs['Alpha'],threshold.inputs[0]);transparent=ns.new('ShaderNodeBsdfTransparent');mix=ns.new('ShaderNodeMixShader')
                    ls.new(threshold.outputs[0],mix.inputs[0]);ls.new(transparent.outputs[0],mix.inputs[1]);ls.new(shader,mix.inputs[2]);shader=mix.outputs[0]
                out=next(n for n in ns if n.type=='OUTPUT_MATERIAL' and n.is_active_output);ls.new(shader,out.inputs['Surface'])
                variants[material.name][mode]=m
    for obj in scene.objects:
        if obj.type=='MESH':obj.hide_render=True
    scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=1;scene.cycles.use_denoising=False
    scene.cycles.transparent_max_bounces=128
    scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG';scene.render.film_transparent=True
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.view_settings.exposure=0;scene.view_settings.gamma=1
    camera=bpy.data.objects.new('Grouping review camera',bpy.data.cameras.new('Grouping review camera'));scene.collection.objects.link(camera)
    camera.data.type='ORTHO';camera.data.clip_end=20000;scene.camera=camera
    for group in plan['groups']:
        if not group['changed']:continue
        if only and group['id'] not in only:continue
        folder=root/'packets'/group['id'];folder.mkdir(parents=True,exist_ok=True)
        selected=[bpy.data.objects[n] for n in group['objects']];points=world_points(selected)
        for o in selected:o.hide_render=False
        entries=[]
        for index,(azimuth,elevation) in enumerate([(0,35),(-55,35),(55,35),(180,35)]):
            a,x=math.radians(azimuth),math.radians(90-elevation)
            rx=np.array([[1,0,0],[0,math.cos(x),-math.sin(x)],[0,math.sin(x),math.cos(x)]])
            rz=np.array([[math.cos(a),-math.sin(a),0],[math.sin(a),math.cos(a),0],[0,0,1]])
            rotation=rz@rx;local=points@rotation;lo,hi=local.min(0),local.max(0)
            center=(lo+hi)/2;center[2]=hi[2]+1500
            camera.location=rotation@center;camera.rotation_euler=(x,0,a)
            camera.data.ortho_scale=max(hi[:2]-lo[:2])*1.12
            bpy.context.view_layer.update()
            entries.append(dict(index=index,azimuth_degrees=azimuth,elevation_degrees=elevation,
                camera_matrix_world=[list(row) for row in camera.matrix_world],ortho_scale=camera.data.ortho_scale,
                crop=dict(left=index%2*512,top=index//2*512,width=512,height=512)))
            for mode in ('solid','textured'):
                for obj in selected:
                    for slot,original in enumerate(original_slots[obj.name]):obj.data.materials[slot]=variants[original.name][mode]
                scene.render.filepath=str(folder/f'{mode}-{index}.png');bpy.ops.render.render(write_still=True)
        for mode in ('solid','textured'):
            sheet=Image.new('RGBA',(1024,1024))
            for index in range(4):
                with Image.open(folder/f'{mode}-{index}.png') as tile:sheet.paste(tile,(index%2*512,index//2*512))
            sheet.save(folder/(mode+'.png'))
        uv=np.column_stack([points[:,0],-points[:,1]*math.sin(math.radians(35))-points[:,2]*math.cos(math.radians(35))])
        lo=np.floor(uv.min(0)-20).astype(int);hi=np.ceil(uv.max(0)+20).astype(int)
        bounds=[int(max(0,lo[0])),int(max(0,lo[1])),int(min(1920,hi[0])),int(min(1088,hi[1]))]
        with Image.open(EDITOR/'work/needs-review/sherwood-refinement/textures/original-static-art.png') as art:art.crop(bounds).save(folder/'original.png')
        packet={**group,'scope':'grouping-only','geometry_uv_materials_unchanged':True,
            'worker_sha256':receipt['worker_sha256'],'catalog_sha256':receipt['catalog_sha256'],'views':entries,'source_bounds':bounds,
            'files':{name:sha(folder/name) for name in ('solid.png','textured.png','original.png')}}
        (folder/'packet.json').write_text(json.dumps(packet,indent=2)+'\n')
        for o in selected:o.hide_render=True
        print('GROUP READY '+group['id'],flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',required=True);p.add_argument('--asset',action='append')
    a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);main(a.root,a.asset)
