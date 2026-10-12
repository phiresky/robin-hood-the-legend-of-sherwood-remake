"""Render a revised asset with the original camera first and original art beside it."""
import argparse
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image

HERE=Path(__file__).resolve().parent
EDITOR=HERE.parents[1]
sys.path[:0]=[str(HERE),str(EDITOR/'refinement')]
from render_slots import acquire
from stage_grouping_review import world_points,sha


def main(root,asset):
    root=Path(root).resolve();receipt=json.loads((root/'stage.json').read_text())
    assert sha(root/'candidate.blend')==receipt['worker_sha256']
    acquire();bpy.ops.wm.open_mainfile(filepath=str(root/'candidate.blend'))
    scene=bpy.data.scenes['Sherwood Editor Migration'];bpy.context.window.scene=scene
    selected=[o for o in bpy.data.collections['Sherwood Working'].objects if o.type=='MESH' and o.get('asset_group')==asset]
    if not selected:raise ValueError('Missing revised asset: '+asset)
    originals={o.name:list(o.data.materials) for o in selected};variants={}
    for obj in selected:
        for material in originals[obj.name]:
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
        if obj.type=='MESH':obj.hide_render=obj not in selected
    scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=4;scene.cycles.use_denoising=False
    scene.cycles.transparent_max_bounces=128
    scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG';scene.render.film_transparent=True
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.view_settings.exposure=0;scene.view_settings.gamma=1
    camera=bpy.data.objects.new('Model review camera',bpy.data.cameras.new('Model review camera'));scene.collection.objects.link(camera)
    camera.data.type='ORTHO';camera.data.clip_end=20000;scene.camera=camera
    folder=root/'packet';folder.mkdir(exist_ok=True)
    points=world_points(selected);entries=[]
    for index,(azimuth,elevation) in enumerate([(0,35),(-55,35),(55,35),(180,35)]):
        a,x=math.radians(azimuth),math.radians(90-elevation)
        rx=np.array([[1,0,0],[0,math.cos(x),-math.sin(x)],[0,math.sin(x),math.cos(x)]])
        rz=np.array([[math.cos(a),-math.sin(a),0],[math.sin(a),math.cos(a),0],[0,0,1]])
        rotation=rz@rx;local=points@rotation;lo,hi=local.min(0),local.max(0)
        center=(lo+hi)/2;center[2]=hi[2]+1500
        camera.location=rotation@center;camera.rotation_euler=(x,0,a);camera.data.ortho_scale=max(hi[:2]-lo[:2])*1.12
        bpy.context.view_layer.update()
        entries.append(dict(index=index,azimuth_degrees=azimuth,elevation_degrees=elevation,
            camera_matrix_world=[list(row) for row in camera.matrix_world],ortho_scale=camera.data.ortho_scale,
            crop=dict(left=index%2*512,top=index//2*512,width=512,height=512)))
        for mode in ('solid','textured'):
            for obj in selected:
                for slot,original in enumerate(originals[obj.name]):obj.data.materials[slot]=variants[original.name][mode]
            scene.render.filepath=str(folder/f'{mode}-{index}.png');bpy.ops.render.render(write_still=True)
    for mode in ('solid','textured'):
        sheet=Image.new('RGBA',(1024,1024))
        for index in range(4):
            with Image.open(folder/f'{mode}-{index}.png') as tile:sheet.paste(tile,(index%2*512,index//2*512))
        sheet.save(folder/(mode+'.png'))
    screen=np.column_stack([points[:,0],-points[:,1]*math.sin(math.radians(35))-points[:,2]*math.cos(math.radians(35))])
    lo=np.floor(screen.min(0)-20).astype(int);hi=np.ceil(screen.max(0)+20).astype(int)
    bounds=[max(0,int(lo[0])),max(0,int(lo[1])),min(1920,int(hi[0])),min(1088,int(hi[1]))]
    art=EDITOR/'work/needs-review/sherwood-refinement/textures/original-static-art.png'
    with Image.open(art) as original:original.crop(bounds).save(folder/'original.png')
    day=EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'
    with Image.open(day) as original:original.crop(bounds).save(folder/'original-day.png')
    packet=dict(asset_id=asset,scope='model-review',worker_sha256=receipt['worker_sha256'],views=entries,
        source_bounds=bounds,original_art_sha256=sha(art),original_day_sha256=sha(day),
        files={name:sha(folder/name) for name in ('solid.png','textured.png','original.png','original-day.png')})
    (folder/'packet.json').write_text(json.dumps(packet,indent=2)+'\n')
    print('MODEL PACKET RENDERED '+asset,flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',required=True);p.add_argument('--asset',required=True)
    a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);main(a.root,a.asset)
