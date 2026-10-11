"""Render a private geometry-only trial in eight original-camera-first views."""
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
R = ROOT/'level-editor/work/croisement01-refinement/restart2'
packet=R/sys.argv[sys.argv.index('--')+1]
assert packet.resolve().parent==R.resolve()
close='--close' in sys.argv
normal_trial='--smooth-join-normals' in sys.argv
out=packet/(('solid-close-review-v1' if close else 'solid-review-v1')+('-join-normals' if normal_trial else ''))
assert not out.exists()
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free>=10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
    data=np.load(packet/'candidate.npz')
    vertices,faces=data['vertices'],data['faces']
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene=bpy.context.scene
    center=(vertices.min(0)+vertices.max(0))/2
    mesh=bpy.data.meshes.new('Private branch anatomy')
    mesh.from_pydata((vertices-center).tolist(),[],faces.tolist())
    mesh.update()
    for face in mesh.polygons:face.use_smooth=True
    obj=bpy.data.objects.new(mesh.name,mesh)
    scene.collection.objects.link(obj)
    obj.location=Vector(center)
    if normal_trial:
        from scipy.spatial import cKDTree
        ns=np.array([p.normal[:] for p in mesh.vertices]);result=ns.copy()
        sy,cy=math.sin(math.radians(35)),math.cos(math.radians(35));native=-vertices[:,1]*sy-vertices[:,2]*cy
        def smooth(t):
            t=np.clip(t,0,1);return t*t*(3-2*t)
        weight=smooth((native-275)/20)*smooth((365-native)/20)*smooth((vertices[:,0]-510)/15)*smooth((630-vertices[:,0])/15)
        tree=cKDTree(vertices)
        for j in np.flatnonzero(weight):
            ids=np.array(tree.query_ball_point(vertices[j],7.));delta=vertices[ids]-vertices[j]
            w=np.exp(-np.sum(delta*delta,axis=1)/18)*(ns[ids]@ns[j]>.6)
            mean=(ns[ids]*w[:,None]).sum(0);mean/=np.linalg.norm(mean)
            result[j]=ns[j]*(1-weight[j])+mean*weight[j];result[j]/=np.linalg.norm(result[j])
        mesh.normals_split_custom_set_from_vertices(result.tolist());mesh.update()

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
    if close:
        center=np.array([560.,-650.,115.]);camera_data.ortho_scale=190.
    scene.render.engine='BLENDER_WORKBENCH'
    scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE'
    scene.display.shading.single_color=(.5,.5,.5)
    scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True
    out.mkdir()
    sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
    for i,direction in enumerate(directions):
        camera.location=Vector(center)+direction*1800
        camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler()
        scene.render.filepath=str(out/f'solid-{i}.png')
        bpy.ops.render.render(write_still=True)
        sheet.paste(Image.open(out/f'solid-{i}.png').convert('RGB'),(i%4*384,i//4*408+24))
        draw.text((i%4*384+5,i//4*408+5),f'{i}'+(' original art camera' if i==0 else ''),fill='white')
    sheet.save(out/'sheet.png')
    (out/'receipt.json').write_text(json.dumps(dict(status='PRIVATE_SOLID_REVIEW_ONLY',candidate_sha256=sha(packet/'candidate.npz'),sheet_sha256=sha(out/'sheet.png'),scope='Actual trial mesh geometry; no texture, contact or intersection approval implied.'),indent=2)+'\n')
finally:
    release()
