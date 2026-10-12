"""Save connected private wood with the retained native-art diagnostic material."""
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys
import bpy
import numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from review_evidence import sha
from restart2_tree08_union import topology
R=ROOT/'level-editor/work/active/croisement01/restart2'
packet=R/'tree08-connected-tip-cleanup-v2'
parent=R/'tree08-wood-prototype-v17-bank-bands/model.blend'
out=R/'tree08-wood-prototype-v18-connected'
assert not out.exists()
guard=json.loads((packet/'source-guard.json').read_text())
assert guard['status']=='PASS_SOURCE_RAYS_AND_TOPOLOGY'
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free>=10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
    parent_sha=sha(parent);candidate_sha=sha(packet/'candidate.npz')
    bpy.ops.wm.open_mainfile(filepath=str(parent))
    packed={im.name:hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file}
    obj=next(o for o in bpy.context.scene.objects if o.type=='MESH')
    oldmesh=obj.data;materials=list(oldmesh.materials)
    data=np.load(packet/'candidate.npz');vertices,faces=data['vertices'],data['faces']
    # Subtract in float64 before storing local float32 coordinates. Casting
    # world coordinates through Vector first destroys tiny valid union edges.
    origin=np.array([552.,-672.,235.])
    obj.matrix_world=Matrix.Translation(Vector(origin))
    mesh=bpy.data.meshes.new('Connected Tree08 wood')
    mesh.from_pydata((vertices-origin).tolist(),[],faces.tolist())
    for material in materials:mesh.materials.append(material)
    obj.data=mesh;bpy.data.meshes.remove(oldmesh)
    for face in mesh.polygons:face.use_smooth=True
    uv=mesh.uv_layers.new(name='Native source projection')
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    ray,down=Vector((0,-c,s)),Vector((0,-s,-c))
    for loop in mesh.loops:
        p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=((p.x-331)/446,1-(-p.y*s-p.z*c-11)/461)
    mesh.update();bpy.context.view_layer.update()
    check=topology(obj)
    assert check['components']==1
    assert not any(check[k] for k in ['zero_area_triangles','nonmanifold_edges','inconsistent_edge_winding'])
    out.mkdir();bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
    bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'))
    assert packed=={im.name:hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file}
    scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH')
    world=[obj.matrix_world@p.co for p in obj.data.vertices]
    bvh=BVHTree.FromPolygons(world,[p.vertices[:] for p in obj.data.polygons])
    core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
    for y,x in np.argwhere(core):
        assert bvh.ray_cast(Vector((x+331.5,0,0))+down*(y+11.5)+ray*2000,-ray,4000)[0] is not None
    scene.render.threads_mode='FIXED';scene.render.threads=2
    scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.device='CPU';scene.cycles.use_denoising=False
    scene.render.resolution_x=446;scene.render.resolution_y=461;scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG'
    camera_data=bpy.data.cameras.new('Native art diagnostic camera');camera=bpy.data.objects.new(camera_data.name,camera_data)
    scene.collection.objects.link(camera);scene.camera=camera
    camera_data.type='ORTHO';camera_data.ortho_scale=461;camera_data.clip_end=10000
    camera.location=Vector((554,0,0))+down*241.5+ray*1500
    camera.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler()
    scene.render.filepath=str(out/'native.png');bpy.ops.render.render(write_still=True)
    expected=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal-rgba.png').convert('RGBA'))
    actual=np.asarray(Image.open(out/'native.png').convert('RGBA'))
    difference=np.abs(expected[core,:3].astype(int)-actual[core,:3].astype(int))
    assert sha(parent)==parent_sha and sha(packet/'candidate.npz')==candidate_sha
    report=dict(status='PRIVATE_CONNECTED_GEOMETRY_ANATOMY_STILL_HOLD',model_sha256=sha(out/'model.blend'),candidate_sha256=candidate_sha,parent_sha256=parent_sha,topology=check,source_pixels=int(core.sum()),antialiased_render_changed_pixels=int(np.any(difference!=0,axis=1).sum()),antialiased_render_max_error=int(difference.max()),packed_source_images_exact=True,limits=['Native-art projection is diagnostic, not final ownership-aware texture baking.','Full strict intersections, actual bank contact review and acceptable branch anatomy remain required.','No geometry approval, synthesis or publication implied.'])
    (out/'saved-receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
finally:
    release()
