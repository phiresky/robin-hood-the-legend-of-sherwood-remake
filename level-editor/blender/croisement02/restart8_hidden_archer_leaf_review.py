"""Reopen private leaf endpoints for complete alpha coverage and native-first views."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'),
               str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_review import _tree
from tree_geometry import SIN, COS, RAY


def main():
    root = OUT / 'restart8-hidden-archer-leaf-trial-v1'
    for folder in sorted(root.glob('profile-*')):
        record = json.loads((folder / 'construction.json').read_text())
        model = folder / 'model.blend'
        assert sha(model) == record['model_sha256']
        destination = folder / 'review-v2'
        destination.mkdir(exist_ok=False)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        objects = [o for o in scene.objects if o.type == 'MESH']
        tree, owners, _ = _tree(objects)
        source = np.array(Image.open(record['source']).convert('RGBA'))
        h,w = source.shape[:2]
        x0,y0 = record['source_top_left']
        missing, extra, z = [], [], []
        for y in range(-2,h+2):
            for x in range(-2,w+2):
                origin = Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000
                point,normal,index,distance = tree.ray_cast(origin,-RAY)
                expected = 0<=y<h and 0<=x<w and source[y,x,3]>=128
                if expected and point is None: missing.append([x0+x,y0+y])
                if not expected and point is not None: extra.append([x0+x,y0+y])
                if expected and point is not None: z.append(float(point.z))
        write_json(destination/'native-coverage.json',dict(model_sha256=sha(model),
            source_sha256=record['source_sha256'], expected_opaque=record['source_opaque_centers'],
            missing=missing,extra=extra,occupied_height_range=[min(z),max(z)],
            limitations=['Independent silhouette center test, not color or neighbor support proof.']))
        scene.render.engine='CYCLES'
        scene.cycles.samples=8
        scene.cycles.use_denoising=False
        scene.cycles.transparent_max_bounces=256
        scene.render.resolution_x=scene.render.resolution_y=320
        scene.render.resolution_percentage=100
        scene.render.image_settings.file_format='PNG'
        scene.render.film_transparent=False
        scene.world=bpy.data.worlds.new('Neutral leaf inspection')
        scene.world.use_nodes=True
        scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.22,.22,.22,1)
        scene.world.node_tree.nodes['Background'].inputs[1].default_value=.7
        scene.view_settings.view_transform='Standard'
        camera=bpy.data.objects.new('Native-first camera',bpy.data.cameras.new('Native-first camera'))
        scene.collection.objects.link(camera)
        scene.camera=camera
        camera.data.type='ORTHO'
        camera.data.clip_start=.1
        camera.data.clip_end=10000
        points=np.array([o.matrix_world@v.co for o in objects for v in o.data.vertices])
        low,high=points.min(0),points.max(0)
        center=Vector((low+high)/2)
        camera.data.ortho_scale=float(np.linalg.norm(high-low))*1.12
        saved=[]
        for mat in bpy.data.materials:
            if not mat.use_nodes:continue
            for node in mat.node_tree.nodes:
                if node.type=='BSDF_PRINCIPLED':
                    for key in ['Base Color','Emission Color']:
                        socket=node.inputs[key]
                        if socket.links:saved.append((mat,node,key,socket.links[0].from_socket))
        for mode in ['actual','solid']:
            if mode=='solid':
                for mat,node,key,from_socket in saved:
                    for link in list(node.inputs[key].links):mat.node_tree.links.remove(link)
                    node.inputs[key].default_value=(.35,.35,.35,1)
            images=[]
            for view in range(8):
                a=view*math.pi/4
                direction=Vector((COS*math.sin(a),-COS*math.cos(a),SIN))
                camera.location=center+direction*4000
                camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
                path=destination/f'{mode}-{view}.png'
                scene.render.filepath=str(path)
                bpy.ops.render.render(write_still=True)
                image=Image.open(path).convert('RGB')
                assert np.asarray(image).std() > 2, 'Blank or clipped asset render'
                images.append(image)
            sheet=Image.new('RGB',(1280,680),(25,25,25))
            draw=ImageDraw.Draw(sheet)
            for i,image in enumerate(images):
                left,top=(i%4)*320,(i//4)*340
                sheet.paste(image,(left,top+20));draw.text((left+8,top+3),'Original camera' if i==0 else f'View{i+1}',fill='white')
            sheet.save(destination/f'{mode}-eight.png')
        assert sha(model)==record['model_sha256']
        write_json(destination/'evidence.json',dict(model_sha256=sha(model),
            native_camera_direction=list(RAY),native_first=True,
            actual_sheet_sha256=sha(destination/'actual-eight.png'),
            solid_sheet_sha256=sha(destination/'solid-eight.png'),
            coverage_sha256=sha(destination/'native-coverage.json'),
            status='Private actual/solid review ready for author inspection; support still pending'))


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
