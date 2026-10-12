"""Private crown geometry from exact isolated native animation artwork."""
import json
import shutil
import sys
import math
from pathlib import Path

import bpy
import numpy as np
from PIL import Image
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender'),
                str(ROOT/'level-editor/blender/leicester')]
from render_slots import acquire
from evidence_io import sha
from render_multiview_asset import render
from setup_map import fit_camera
import foliage_trees as foliage

acquire(slots=2)
assert shutil.disk_usage(ROOT).free > 10*1024**3 + 512*1024**2
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                if x.startswith('MemAvailable:')))*1024 > 6*1024**3
base = ROOT/'level-editor/work/active/croisement01/restart2'
wood = base/'tree14-wood-v16-projected/assets/croisement01-tree-14'
out = base/'tree14-crown94-geometry-v6'
assert not out.exists()
out.mkdir()
model = wood/'model.blend'
wood_phase = json.loads((wood.parents[1]/'phase-modified.json').read_text())
assert wood_phase['phase'] == 'modified'
assert sha(model) == wood_phase['model_sha256']
bpy.ops.wm.open_mainfile(filepath=str(model))
config = json.loads((wood/'workspace.json').read_text())
scene = bpy.data.scenes[config['scene_name']]
bpy.context.window.scene = scene
wood_objects = [o for o in scene.objects if o.type == 'MESH' and o.get('asset_group') == config['asset_id']]
protected = {o.name: foliage.geometry_hash(o) for o in wood_objects}
# Seeds follow visible source clusters, not another tree's layout.
foliage.CONFIG[94] = dict(mask=94, ground=412., canopy_bottom=343,
    seeds=[(780,90),(825,130),(884,155),(725,165),(795,205),(915,215),(720,235),(850,285)])
source_packet = base/'tree14-native-crown-animation-source-v3/source-partition.json'
evidence = json.loads(source_packet.read_text())
assert evidence['native_animation'] == 1 and evidence['native_frame'] == 0
assert evidence['canopy_pixels'] == 16719
assert evidence['all_fourteen_frame_rgba_unions_exact']
assert evidence['animated_union_pixels'] == 19393
for lobe in evidence['lobes']:
    for field in ('source','unknown'):
        assert sha(Path(lobe[field])) == lobe[field+'_sha256']
    for frame in lobe['frames']:
        assert sha(Path(frame['path'])) == frame['sha256']
for frame in evidence['frame_records']:
    assert sha(ROOT/frame['path']) == frame['sha256']
(out/'source-partition.json').write_text(json.dumps(evidence,indent=2)+'\n')
for lobe in evidence['lobes']:
    lobe['depth_radius'] = 210.
mesh = bpy.data.meshes.new('Crown94 provisional volume')
crown = bpy.data.objects.new('Tree14 Crown94 provisional',mesh)
bpy.data.collections[config['collection_name']].objects.link(crown)
crown['asset_group'] = config['asset_id']
crown['source_node'] = 'foliage-tree14-crown94'
crown['projection_component'] = 'crown'
crown['ownership_status'] = 'Exact native frame-0 leaf RGBA; inferred geometry requires joint first-hit review'
report = foliage.refine_crown(crown,94,evidence)
# Put the paired patches on one rounded envelope. Giving each small patch the
# whole crown's depth makes elongated tubes instead of a coherent leaf volume.
center=Vector((814.5,-412/foliage.SINE,(412-193)/foliage.COSINE))
radii=Vector((185.,195.,185.))
ray=foliage.RAY
def surface(x,y,front):
    plane=Vector((x,-412/foliage.SINE,(412-y)/foliage.COSINE))
    delta=plane-center
    aa=sum((ray[i]/radii[i])**2 for i in range(3))
    bb=2*sum(delta[i]*ray[i]/radii[i]**2 for i in range(3))
    cc=sum((delta[i]/radii[i])**2 for i in range(3))-1
    discriminant=max(0.,bb*bb-4*aa*cc)
    # Broad uneven masses avoid a uniformly spherical reverse outline. This
    # displacement follows source rays, so it cannot move painted leaf texels.
    fullness=.96+.07*math.sin((x-649)*.037+(y-43)*.019)+.035*math.cos((y-43)*.051-(x-649)*.015)
    depth=(-bb+(1 if front else -1)*math.sqrt(discriminant)*fullness)/(2*aa)
    return plane+ray*depth
cursor=0
for lobe in evidence['lobes']:
    x0,y0,x1,y1=lobe['bbox_source']
    for front in (True,False):
        for j in range(9):
            for i in range(9):
                crown.data.vertices[cursor].co=surface(x0+(x1-x0)*i/8,y0+(y1-y0)*j/8,front)
                cursor+=1
    assert not lobe.get('front_unknown')
    for side in (-1,1):
        for v in (0.,1.):
            for u in (0.,1.):
                x=(x0+x1)/2+side*.1;y=y0+(y1-y0)*v
                crown.data.vertices[cursor].co=surface(x,y,bool(u))
                cursor+=1
assert cursor==len(crown.data.vertices)
crown.data.update()
report['after_geometry_sha256']=foliage.geometry_hash(crown)
report['envelope_radii']=list(radii)
# Interior fixed cross-sections are edge-on to the source camera. Subdivision
# follows the shared envelope instead of spanning it with a single corner quad.
vertices=[];faces=[];uvs=[];slots=[];materials=[]
steps=16
for lobe in evidence['lobes']:
    x0,y0,x1,y1=lobe['bbox_source']
    slot=len(materials)
    materials.append(foliage.material('Crown94 inferred interior '+str(slot),lobe['unknown'],False,evidence,two_sided=True))
    for fraction in (.2,.5,.8):
        x=x0+(x1-x0)*fraction;start=len(vertices)
        for j in range(steps+1):
            v=j/steps;y=y0+(y1-y0)*v
            back,front=surface(x,y,False),surface(x,y,True)
            for i in range(steps+1):
                u=i/steps;vertices.append(back.lerp(front,u));uvs.append((u,1-v))
        for j in range(steps):
            for i in range(steps):
                a=start+j*(steps+1)+i;b=a+1;c=b+steps+1;d=a+steps+1
                if (vertices[b]-vertices[a]).length<1e-6 or (vertices[c]-vertices[d]).length<1e-6:continue
                faces.extend(((a,b,c),(a,c,d)));slots.extend((slot,slot))
    # A second family prevents the exact reverse camera seeing all interiors
    # edge-on simultaneously. These remain edge-on to the original camera.
    for fraction in (.2,.5,.8):
        y=y0+(y1-y0)*fraction;start=len(vertices)
        for j in range(steps+1):
            v=j/steps;x=x0+(x1-x0)*v
            back,front=surface(x,y,False),surface(x,y,True)
            for i in range(steps+1):
                u=i/steps;vertices.append(back.lerp(front,u));uvs.append((u,1-v))
        for j in range(steps):
            for i in range(steps):
                a=start+j*(steps+1)+i;b=a+1;c=b+steps+1;d=a+steps+1
                if (vertices[b]-vertices[a]).length<1e-6 or (vertices[c]-vertices[d]).length<1e-6:continue
                faces.extend(((a,b,c),(a,c,d)));slots.extend((slot,slot))
interior_mesh=bpy.data.meshes.new('Crown94 inferred interior fixed foliage')
interior_mesh.from_pydata(vertices,[],faces);interior_mesh.update()
for material in materials:interior_mesh.materials.append(material)
uv_layer=interior_mesh.uv_layers.new(name='Foliage UV')
ownership=interior_mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
interior_mesh.color_attributes.active_color=ownership
for polygon,slot in zip(interior_mesh.polygons,slots):
    polygon.material_index=slot
    for loop in polygon.loop_indices:
        uv_layer.data[loop].uv=uvs[interior_mesh.loops[loop].vertex_index]
        ownership.data[loop].color=(0.,1.,1.,1.)
interior=bpy.data.objects.new('Tree14 Crown94 inferred interior',interior_mesh)
bpy.data.collections[config['collection_name']].objects.link(interior)
interior['asset_group']=config['asset_id'];interior['source_node']='foliage-tree14-crown94-interior'
interior['projection_component']='crown';interior['foliage_physical_opacity']=True
report['interior_geometry_sha256']=foliage.geometry_hash(interior)
assert protected == {o.name: foliage.geometry_hash(o) for o in wood_objects}
points = [crown.matrix_world@v.co for v in crown.data.vertices]
extent = [max(p[i] for p in points)-min(p[i] for p in points) for i in range(3)]
assert extent[1] >= extent[0], extent
scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
saved = out/'model.blend'
bpy.ops.wm.save_as_mainfile(filepath=str(saved))
packet=json.loads((wood/'input/views.json').read_text())
packet['object_names']=[o.name for o in wood_objects]+[crown.name,interior.name]
packet['render_object_names']=None
data=bpy.data.cameras.new('Joint crown diagnostic');data.type='ORTHO'
camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
for view in packet['views']:
    view['crop']={'width':384,'height':384}
    camera.matrix_world=Matrix(view['camera_matrix_world'])
    fit_camera(camera,wood_objects+[crown,interior],1.,padding=1.2)
    view['camera_matrix_world']=[list(row) for row in camera.matrix_world]
    view['ortho_scale']=data.ortho_scale
manifest=out/'views.json';manifest.write_text(json.dumps(packet,indent=2)+'\n')
report.update(status='DIAGNOSTIC_NOT_READY_FOR_APPROVAL',model_sha256=sha(saved),
    wood_model_sha256=sha(model),wood_geometry_unchanged=True,extent_xyz=extent,
    limitations=['Crown94 association remains a joint source review hypothesis.',
      'Native leaf first-hit coverage and its association with retained wood need joint review.',
      'Fourteen-frame motion is preserved as source evidence but not integrated in this geometry diagnostic.',
      'Reverse and transverse neutral coverage and depth are inferred, not generated textures.'],
    texture_synthesis=False)
(out/'checkpoint.json').write_text(json.dumps(report,indent=2)+'\n')
render(manifest,out/'material-views',modes=('textured',),width=256)
images=[Image.open(out/f'material-views/view-{i}-textured.png').convert('RGB') for i in range(8)]
width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
sheet.save(out/'material-eight.png')
