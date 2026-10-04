"""Read-only vertical contact audit against the strictly selected woodland bank."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json
from tree_geometry import SIN,COS
from render_slots import acquire,release


def main():
    worker=scenery_workspace('croisement02-north-woodland-bank')
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    vertices=[];faces=[]
    for obj in bpy.data.objects:
        if obj.type!='MESH' or obj.get('asset_group')!=worker.name:continue
        offset=len(vertices);vertices.extend([obj.matrix_world@v.co for v in obj.data.vertices])
        faces.extend([tuple(offset+i for i in p.vertices) for p in obj.data.polygons])
    if not vertices:raise ValueError('Bank geometry absent')
    bvh=BVHTree.FromPolygons(vertices,faces)
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());records=[]
    for index in range(117,124):
        row=level['masks'][index];x,y=row['box_top_left'];alpha=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'))>0
        yy,xx=np.nonzero(alpha);height=float(yy.max()-yy.min()+1)
        root=Vector((x+(xx.min()+xx.max()+1)/2,-(y+yy.max()+1-height*.18+36)/SIN,36/COS+.05))
        hit,normal,face,distance=bvh.ray_cast(root+Vector((0,0,200)),Vector((0,0,-1)),400)
        records.append(dict(native_mask=index,root_world=list(root),terrain_hit=list(hit) if hit is not None else None,
                            vertical_gap=float(root.z-hit.z) if hit is not None else None))
    write_json(OUT/'ground-plant-candidates/bank-contact-audit.json',dict(
        bank_workspace=str(worker),bank_model_sha256=sha(worker/'model.blend'),records=records,
        status='measurement; root placement must match an actual bank surface before integration'))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
