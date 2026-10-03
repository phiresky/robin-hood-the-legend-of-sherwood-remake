"""Independent raster comparison of saved tree geometry against its source masks."""
import json
import math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image
from catalog import OUT
from evidence_io import sha
from tree_geometry import SIN,COS,RAY


def audit(workspace,objects):
    report=json.loads((workspace/'inspection/refinement.json').read_text())
    if 'mask' not in report:return None
    row=next(r for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==report['mask'])
    packet=json.loads(Path(row['packet']).read_text());expected=np.zeros((1152,1792),dtype=bool)
    def paste(alpha,x,y):
        h,w=alpha.shape;left,top=max(0,x),max(0,y);right,bottom=min(1792,x+w),min(1152,y+h)
        if right>left and bottom>top:expected[top:bottom,left:right]|=alpha[top-y:bottom-y,left-x:right-x]
    x,y,_,_=packet['native_bbox'];paste(np.asarray(Image.open(Path(row['packet']).parent/'complete-source.png'))[:,:,3]>127,x,y)
    native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==report['mask'])
    paste(np.asarray(Image.open(OUT/'baseline/masks'/native['png']).convert('L'))>0,*native['box_top_left'])
    yy,xx=np.nonzero(expected);left=max(0,int(xx.min())-10);right=min(1792,int(xx.max())+11);top=max(0,int(yy.min())-10);bottom=min(1152,int(yy.max())+11)
    width,height=right-left,bottom-top;expected=expected[top:bottom,left:right]
    scene=bpy.data.scenes.new('Isolated source coverage audit');copies=[]
    for obj in objects:
        copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False
        scene.collection.objects.link(copy);copies.append(copy)
    target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Exact source coverage')
    data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64
    scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
    scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA'
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    destination=workspace/'inspection/source-coverage';destination.mkdir(exist_ok=True);scene.render.filepath=str(destination/'render.png')
    bpy.ops.render.render(write_still=True,scene=scene.name)
    actual=np.asarray(Image.open(destination/'render.png').convert('RGBA'))[:,:,3]>127
    missing=expected&~actual;extra=actual&~expected;intersection=expected&actual
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((left,top,right,bottom));source.save(destination/'source.png')
    overlay=np.asarray(source).copy();overlay[missing]=[255,40,40];overlay[extra]=[0,220,255];Image.fromarray(overlay).save(destination/'difference.png')
    Image.fromarray(expected.astype('uint8')*255).save(destination/'expected.png')
    result=dict(model_sha256=sha(workspace/'model.blend'),source_packet_sha256=sha(row['packet']),source_crop=[left,top,right,bottom],expected_pixels=int(expected.sum()),rendered_pixels=int(actual.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),intersection_over_union=float(intersection.sum()/np.count_nonzero(expected|actual)),legend='Red: native coverage missed. Cyan: rendered coverage outside assigned native masks. Crossed foliage edges and mask-derived wood thickness can differ.',status='measurement; requires visual review')
    (destination/'report.json').write_text(json.dumps(result,indent=2)+'\n')
    for obj in copies+[camera]:bpy.data.objects.remove(obj,do_unlink=True)
    bpy.data.cameras.remove(data);bpy.data.scenes.remove(scene)
    return result
