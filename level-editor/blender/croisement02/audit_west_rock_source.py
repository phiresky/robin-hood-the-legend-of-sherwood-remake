"""Measure saved rock geometry coverage against its observed source domain."""
import json
import argparse
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from tree_geometry import SIN, RAY
from render_slots import acquire, release


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--asset',default='croisement02-west-rock-outcrop')
    parser.add_argument('--domain',type=Path,default=OUT/'west-rock-source-revision/domain-350.png')
    parser.add_argument('--crop',nargs=4,type=int,default=[0,280,280,440])
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    worker=scenery_workspace(args.asset)
    model_hash=sha(worker/'model.blend')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        scene=bpy.data.scenes.new('Independent rock source coverage')
        white=bpy.data.materials.new('Opaque geometry coverage')
        white.use_nodes=True
        nodes=white.node_tree.nodes;nodes.clear()
        emission=nodes.new('ShaderNodeEmission')
        emission.inputs['Color'].default_value=(1,1,1,1)
        output=nodes.new('ShaderNodeOutputMaterial')
        white.node_tree.links.new(emission.outputs['Emission'],output.inputs['Surface'])
        for original in list(bpy.data.collections['Croisement02 Working'].all_objects):
            if original.type!='MESH' or original.get('asset_group')!=worker.name:continue
            obj=original.copy();obj.data=original.data.copy();obj.parent=None
            obj.matrix_world=original.matrix_world.copy();obj.hide_render=False
            scene.collection.objects.link(obj)
            obj.data.materials.clear();obj.data.materials.append(white)
            for face in obj.data.polygons:face.material_index=0
        box=tuple(args.crop);left,top,right,bottom=box;width=right-left;height=bottom-top
        target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0))
        data=bpy.data.cameras.new('Native rock source camera');data.type='ORTHO'
        data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
        camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
        camera.location=target+RAY*5000
        camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
        scene.camera=camera;scene.render.engine='CYCLES';scene.cycles.samples=8
        scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
        scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG'
        scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        destination=worker/'inspection/source-domain-coverage';destination.mkdir(exist_ok=True)
        scene.render.filepath=str(destination/'render.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        actual=np.asarray(Image.open(destination/'render.png').convert('RGBA'))[:,:,0]>127
        domain=args.domain
        expected=np.asarray(Image.open(domain).convert('L').crop(box))>127
        missing=expected&~actual;extra=actual&~expected
        source=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop(box)).copy()
        source[missing]=[255,40,40];source[extra]=[0,220,255]
        Image.fromarray(source).resize((1120,640),Image.Resampling.NEAREST).save(destination/'difference.png')
        write_json(destination/'report.json',dict(model_sha256=model_hash,domain_sha256=sha(domain),
            expected_pixels=int(expected.sum()),rendered_geometry_pixels=int(actual.sum()),
            missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),
            source_coverage=float((expected&actual).sum()/expected.sum()),
            intersection_over_union=float((expected&actual).sum()/(expected|actual).sum()),
            difference_sha256=sha(destination/'difference.png'),
            legend='Red: observed rock pixels missed by saved geometry. Cyan: geometry outside observed rock domain, including surfaces hidden by foreground plants. This does not measure texture ownership.',
            status='measurement; inspect mismatch locations before readiness'))
        if sha(worker/'model.blend')!=model_hash:raise ValueError('Audit altered saved model')
    finally:release()


if __name__=='__main__':main()
