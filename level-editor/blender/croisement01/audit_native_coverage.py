"""Independent pixel-center geometry coverage against a native occupancy domain."""
import argparse,json,sys,math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from evidence_io import sha

def main():
 p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);p.add_argument('--mask',type=int,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
 w=a.workspace.resolve();out=w/'inspection/native-geometry-coverage'
 if out.exists():raise FileExistsError(out)
 cfg=json.loads((w/'workspace.json').read_text());acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
 objects=[o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id']]
 vertices=[];faces=[]
 for o in objects:
  offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
 tree=BVHTree.FromPolygons(vertices,faces)
 base=ROOT/'level-editor/work/croisement01-refinement/baseline/masks'
 row=next(r for r in json.loads((base/'manifest.json').read_text())['masks'] if r['index']==a.mask)
 expected=np.array(Image.open(base/row['png']).convert('L'))>0
 left,top=row['box_top_left'];height,width=expected.shape;actual=np.zeros_like(expected)
 sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));away=Vector((0,-cosine,sine))
 for y in range(height):
  for x in range(width):
   target=Vector((left+x+.5,-(top+y+.5)/sine,0))
   actual[y,x]=tree.ray_cast(target+away*5000,-away,10000)[0] is not None
 missing=expected&~actual;extra=actual&~expected;match=actual&expected
 colors=np.zeros((height,width,3),dtype=np.uint8);colors[match]=(180,180,180);colors[missing]=(255,50,50);colors[extra]=(0,220,255)
 out.mkdir();Image.fromarray(colors).resize((width*4,height*4),Image.Resampling.NEAREST).save(out/'comparison.png')
 report=dict(status='measurement only; occupancy is not semantic object ownership',model_sha256=sha(w/'model.blend'),mask_sha256=sha(base/row['png']),native_mask=a.mask,expected_pixels=int(expected.sum()),covered_expected_pixels=int(match.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),coverage=float(match.sum()/expected.sum()),scope='Native mask bounding box. Pixel-center rays through saved geometry, independent of materials and alpha.',legend='Gray: native covered; red: native missed; cyan: geometry beyond native occupancy.')
 (out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));release()
if __name__=='__main__':main()
