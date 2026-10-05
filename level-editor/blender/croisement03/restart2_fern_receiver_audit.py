"""Audit which existing scene surfaces receive proposed fern bark pixels."""
import json, math, sys, collections
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    target=OUT/'restart2/fern-receiver-audit-v1';target.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'))
    sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));ray=Vector((0,-cos,sin))
    trees=[]
    for obj in bpy.data.collections['Croisement03 Working'].all_objects:
        if obj.type!='MESH':continue
        vertices=[obj.matrix_world@v.co for v in obj.data.vertices]
        if not vertices:continue
        trees.append((obj,BVHTree.FromPolygons(vertices,[list(p.vertices) for p in obj.data.polygons])))
    rows=[]
    for index,wood in [(35,13),(76,12)]:
        selection=np.asarray(Image.open(OUT/f'restart2/fern-wood-proposal-v1/{index}-proposed-wood.png'))>0
        pixels=[];counts=collections.Counter()
        for y,x in zip(*np.nonzero(selection)):
            origin=Vector((x+.5,-(y+.5)/sin,0))+ray*10000
            hits=[]
            for obj,tree in trees:
                pos,normal,face,distance=tree.ray_cast(origin,-ray)
                if pos is not None:hits.append((distance,obj,pos))
            hits.sort(key=lambda hit:hit[0])
            if hits:
                _,obj,point=hits[0];node=obj.get('source_node',obj.name);counts[node]+=1
                pixels.append(dict(x=int(x),y=int(y),receiver=node,asset=obj.get('asset_group'),point=list(point)))
            else:counts['no-hit']+=1;pixels.append(dict(x=int(x),y=int(y),receiver=None))
        rows.append(dict(fern_mask=index,wood_mask=wood,pixels=pixels,counts=dict(counts)))
    (target/'audit.json').write_text(json.dumps(dict(status='Coarse native scene receivers only; refined trunk geometry and fern joint proof still required.',items=rows),indent=2)+'\n')
    print([(r['fern_mask'],r['counts']) for r in rows]);release()
if __name__=='__main__':main()
