"""Sample the saved mesh's actual UV/image at native pixel-center ray hits."""
import hashlib
import io
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Vector
from mathutils.geometry import barycentric_transform
from mathutils.bvhtree import BVHTree
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
out=R/(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'tree08-wood-prototype-v18-connected')
assert out.resolve().parent==R.resolve()
acquire(slots=2)
try:
    model=out/'model.blend';digest=sha(model)
    bpy.ops.wm.open_mainfile(filepath=str(model))
    obj=next(o for o in bpy.context.scene.objects if o.type=='MESH')
    mesh=obj.data
    textures=[n for m in mesh.materials for n in m.node_tree.nodes if n.type=='TEX_IMAGE']
    assert len(textures)==1 and textures[0].interpolation=='Closest'
    texture=textures[0].image
    actual=np.asarray(Image.open(io.BytesIO(texture.packed_file.data)).convert('RGBA'))
    expected=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal-rgba.png').convert('RGBA'))
    assert np.array_equal(actual,expected)
    core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
    world=[obj.matrix_world@p.co for p in mesh.vertices]
    bvh=BVHTree.FromPolygons(world,[p.vertices[:] for p in mesh.polygons])
    s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
    ray,down=Vector((0,-c,s)),Vector((0,-s,-c))
    mismatches=[];misses=[]
    for y,x in np.argwhere(core):
        point=Vector((x+331.5,0,0))+down*(y+11.5)
        hit=bvh.ray_cast(point+ray*2000,-ray,4000)
        if hit[2] is None:
            misses.append([int(x),int(y)]);continue
        face=mesh.polygons[hit[2]];assert len(face.vertices)==3
        a,b,c=[world[i] for i in face.vertices]
        uv=[Vector((*mesh.uv_layers.active.data[i].uv,0)) for i in face.loop_indices]
        mapped=barycentric_transform(hit[0],a,b,c,*uv)
        sx=int(np.floor(mapped.x*actual.shape[1]));sy=int(np.floor((1-mapped.y)*actual.shape[0]))
        if not (0<=sx<actual.shape[1] and 0<=sy<actual.shape[0]) or not np.array_equal(actual[sy,sx],expected[y,x]):
            mismatches.append(dict(native=[int(x),int(y)],sample=[sx,sy]))
    assert sha(model)==digest
    report=dict(status='PASS_SAVED_NATIVE_CENTER_RGBA' if not misses and not mismatches else 'HOLD_SAVED_NATIVE_RGBA',model_sha256=digest,source_pixels=int(core.sum()),misses=misses,mismatches=mismatches,packed_image_rgba_exact=True,scope='Saved first-hit mesh UV and packed image sampled at6,276native pixel centers. Does not claim antialiased render equality or hidden-surface texture ownership.')
    (out/'saved-native-rgba-guard.json').write_text(json.dumps(report,indent=2)+'\n')
    print(report['status'],len(misses),len(mismatches),flush=True)
finally:
    release()
