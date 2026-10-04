"""Read-only native-camera coverage of inferred wood and fence boundary pixels."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    base=OUT/'understory-candidates/mixed75-91-source-v2';ledger=json.loads((base/'receiver-coverage-ledger.json').read_text())
    destination=base/'receiver-coverage';destination.mkdir(exist_ok=False);results=[]
    for row in ledger['records']:
        if row['receiver']=='ground':
            results.append(dict(row,status='Terrain source-role restoration pending',note='Source ground pixels stay with ground; no foliage geometry claimed'));continue
        mask_path=Path(row['mask'])
        if sha(mask_path)!=row['mask_sha256']:raise ValueError('Boundary mask changed')
        mask=np.asarray(Image.open(mask_path).convert('L'))>0;yy,xx=np.nonzero(mask)
        left,right=max(0,int(xx.min())-4),min(1792,int(xx.max())+5);top,bottom=max(0,int(yy.min())-4),min(1152,int(yy.max())+5)
        worker=tree_workspace(int(row['receiver'][4:])) if row['receiver'].startswith('wood') else scenery_workspace('croisement02-east-upright-rail-fence-95')
        digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        audited=json.loads((worker/'inspection/saved-model-audit.json').read_text())
        objects=[bpy.data.objects[r['object']] for r in audited['objects']]
        objects=[o for o in objects if o.get('projection_component')!='crown']
        if not objects:raise ValueError('No audited receiver wood/fence surfaces')
        scene=bpy.data.scenes.new('Read-only boundary coverage')
        for obj in objects:
            copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy)
        width,height=right-left,bottom-top;target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0))
        camera_data=bpy.data.cameras.new('Exact source boundary camera');camera_data.type='ORTHO';camera_data.sensor_fit='HORIZONTAL';camera_data.ortho_scale=width;camera_data.clip_end=20000
        camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64
        scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100;scene.render.film_transparent=True
        scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        scene.render.filepath=str(destination/f'{row["native_mask"]}-{row["receiver"]}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        alpha=np.asarray(Image.open(scene.render.filepath).convert('RGBA'))[:,:,3]>127;expected=mask[top:bottom,left:right];missing=expected&~alpha
        source=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((left,top,right,bottom))).copy();source[expected&alpha]=[0,230,80];source[missing]=[255,30,30]
        Image.fromarray(source).save(destination/f'{row["native_mask"]}-{row["receiver"]}-coverage.png')
        if sha(worker/'model.blend')!=digest:raise ValueError('Receiver model changed')
        results.append(dict(row,status='Covered by current isolated physical receiver' if not missing.any() else 'HOLD: receiving geometry misses proposed boundary pixels',worker=str(worker),model_sha256=digest,source_crop=[left,top,right,bottom],covered_pixels=int((expected&alpha).sum()),missing_pixels=int(missing.sum()),objects=[o.name for o in objects],limitation='Physical isolated alpha coverage only; source-color refresh and full-scene first-hit ownership remain separate'))
    write_json(destination/'report.json',dict(records=results,read_only=True,approved_models_unchanged=True));print(destination/'report.json')


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
