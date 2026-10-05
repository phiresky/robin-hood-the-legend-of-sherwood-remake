"""Check actual first-hit geometry at the wall's unobserved cap pixels."""
import json,math,sys,collections,hashlib
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    worker=OUT/'restart2/stone-wall-v1/assets/croisement03-southeast-stone-wall';source=OUT/'restart2/southeast-wall-source';audit=json.loads((source/'gray-owner-audit.json').read_text());digest=hashlib.sha256((worker/'model.blend').read_bytes()).hexdigest();assert digest==audit['model_sha256']
    acquire();bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));trees=[]
    for obj in bpy.data.collections['Croisement03 Working'].all_objects:
        if obj.type!='MESH' or (obj.get('asset_group')!='croisement03-southeast-stone-wall' and obj.get('source_node') not in ['building-046','ground']):continue
        tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices) for p in obj.data.polygons]);trees.append((obj,tree))
    sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));ray=Vector((0,-cos,sin));rows=[];counts=collections.Counter()
    for x,y in audit['pixels']:
        origin=Vector((x+.5,-(y+.5)/sin,0))+ray*10000;hits=[]
        for obj,tree in trees:
            point,normal,face,distance=tree.ray_cast(origin,-ray)
            if point is not None:hits.append((distance,obj,point))
        hits.sort(key=lambda h:h[0]);assert hits;distance,obj,point=hits[0];counts[obj['source_node']]+=1;rows.append(dict(x=x,y=y,first_node=obj['source_node'],first_point=list(point),nodes_in_order=[h[1]['source_node'] for h in hits]))
    (source/'gray-first-hit.json').write_text(json.dumps(dict(model_sha256=digest,first_node_counts=dict(counts),pixels=rows,scope='Saved wall plus unchanged coarse tree046 and native ground. This checks current geometric visibility, not completion of the final refined tree.'),indent=2)+'\n');print(dict(counts));release()
if __name__=='__main__':main()
