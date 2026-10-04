"""Compare the isolated post cap against the immutable native-camera baseline."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from complete_fence95_cap import OLD,DEST,ASSET
from catalog import OUT
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    directory=DEST/'source-comparison';directory.mkdir(exist_ok=False);box=[1549,716,1573,740];left,top,right,bottom=box;w,h=right-left,bottom-top
    mask=np.asarray(Image.open(DEST/'new-cap-source.png').convert('L').crop(box))>0;records=[];images=[]
    for label,worker in [('approved',OLD),('candidate',DEST/'assets'/ASSET)]:
        digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));scene=bpy.data.scenes.new('Native post cap comparison')
        for obj in bpy.context.scene.objects:
            if obj.type!='MESH' or obj.get('asset_group')!=ASSET:continue
            copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy)
        target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Native cap camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=w;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.resolution_x=w;scene.render.resolution_y=h;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.filepath=str(directory/f'{label}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        im=Image.open(scene.render.filepath).convert('RGBA');images.append(im);alpha=np.asarray(im)[:,:,3]>127
        if sha(worker/'model.blend')!=digest:raise ValueError('Model changed')
        records.append(dict(label=label,model_sha256=digest,source_pixels=int(mask.sum()),covered_pixels=int((mask&alpha).sum()),missing_pixels=int((mask&~alpha).sum()),render_sha256=sha(Path(scene.render.filepath))))
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(box);panels=[source]
    for im in images:
        bg=Image.new('RGBA',im.size,(80,80,80,255));bg.alpha_composite(im);panels.append(bg)
    for im in images:
        bg=source.copy();bg.alpha_composite(im);panels.append(bg)
    sheet=Image.new('RGB',(w*5,h))
    for i,im in enumerate(panels):sheet.paste(im,(i*w,0))
    sheet.resize((w*5*12,h*12),Image.Resampling.NEAREST).save(directory/'comparison.png')
    write_json(directory/'evidence.json',dict(status='native camera comparison; source-role review pending',source_crop=box,records=records,comparison_sha256=sha(directory/'comparison.png'),source_review_sha256=sha(DEST/'source-review.json')))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
