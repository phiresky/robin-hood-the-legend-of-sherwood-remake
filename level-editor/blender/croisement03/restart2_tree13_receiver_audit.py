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
    target=OUT/'restart2/tree13-wood-v1/receiver-audit';target.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'restart2/tree13-wood-v1/assets/croisement03-tree-13/model.blend'))
    sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));ray=Vector((0,-cos,sin))
    trees=[]
    for obj in bpy.data.collections['Croisement03 Working'].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!='croisement03-tree-13':continue
        vertices=[obj.matrix_world@v.co for v in obj.data.vertices]
        if not vertices:continue
        trees.append((obj,BVHTree.FromPolygons(vertices,[list(p.vertices) for p in obj.data.polygons])))
    rows=[]
    for index,wood in [(35,13)]:
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
    (target/'audit.json').write_text(json.dumps(dict(status='Private revised tree13 wood receiver rays; nine native048 bark pixels required; physical fern joint and mixed foliage still pending.',items=rows),indent=2)+'\n')
    assert rows[0]['counts']=={'building-048':9},rows[0]['counts']
    print([(r['fern_mask'],r['counts']) for r in rows]);release()
if __name__=='__main__':main()
