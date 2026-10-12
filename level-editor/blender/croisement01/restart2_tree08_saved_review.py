"""Review the exact saved private wood model in matching solid/material views."""
import json
import math
from pathlib import Path
import shutil
import sys
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R = ROOT/'level-editor/work/active/croisement01/restart2'
packet=R/sys.argv[sys.argv.index('--')+1]
assert packet.resolve().parent==R.resolve()
out=packet/'saved-review-v1'
assert not out.exists()
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free>=10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
    model_hash=sha(packet/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(packet/'model.blend'))
    scene=bpy.context.scene
    obj=next(o for o in scene.objects if o.type=='MESH')
    vertices=np.array([obj.matrix_world@p.co for p in obj.data.vertices])
    center=(vertices.min(0)+vertices.max(0))/2
    scene.render.threads_mode='FIXED';scene.render.threads=2
    scene.render.resolution_x=scene.render.resolution_y=384
    scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG'
    scene.world=bpy.data.worlds.new('Neutral anatomy world');scene.world.color=(.1,.1,.1)
    camera_data=bpy.data.cameras.new('Original art camera first')
    camera=bpy.data.objects.new(camera_data.name,camera_data)
    scene.collection.objects.link(camera);scene.camera=camera
    camera_data.type='ORTHO';camera_data.clip_end=10000
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    directions=[Vector((c*math.sin(i*math.tau/8),-c*math.cos(i*math.tau/8),s)) for i in range(8)]
    camera_data.ortho_scale=float(np.linalg.norm(vertices.max(0)-vertices.min(0)))*1.06
    scene.render.engine='BLENDER_WORKBENCH'
    scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE'
    scene.display.shading.single_color=(.5,.5,.5)
    scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True
    out.mkdir()
    for mode in ['solid','diagnostic-material']:
        scene.render.engine='BLENDER_WORKBENCH' if mode=='solid' else 'CYCLES'
        if mode!='solid':
            scene.cycles.samples=4;scene.cycles.use_denoising=False;scene.cycles.device='CPU'
        sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
        for i,direction in enumerate(directions):
            camera.location=Vector(center)+direction*1800
            camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler()
            scene.render.filepath=str(out/f'{mode}-{i}.png')
            bpy.ops.render.render(write_still=True)
            sheet.paste(Image.open(out/f'{mode}-{i}.png').convert('RGB'),(i%4*384,i//4*408+24))
            draw.text((i%4*384+5,i//4*408+5),f'{i}'+(' original art camera' if i==0 else ''),fill='white')
        sheet.save(out/f'{mode}-sheet.png')
    assert sha(packet/'model.blend')==model_hash
    (out/'receipt.json').write_text(json.dumps(dict(status='PRIVATE_SAVED_REVIEW_DIAGNOSTIC_MATERIAL',model_sha256=model_hash,solid_sha256=sha(out/'solid-sheet.png'),material_sha256=sha(out/'diagnostic-material-sheet.png'),scope='Exact saved geometry. Projected native artwork is diagnostic; unseen regions are not final ownership-aware texture fill. No geometry approval implied.'),indent=2)+'\n')
finally:
    release()
