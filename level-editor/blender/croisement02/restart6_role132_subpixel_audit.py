"""Read-only subpixel positive-height wood coverage for ambiguous native centers."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 records=json.load(open(OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json'))['records'];inventory=json.load(open(ROOT/'remaining-inventory-v1/report.json'))['rows'];result=[]
 for number in[19,25,95]:
  asset=f'croisement02-tree-{number}'if number!=95 else'croisement02-east-upright-rail-fence-95';r=next(x for x in records if x['asset_id']==asset);source=Path(r['model']);assert sha(source)==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and'Crown'not in o.name];tree,owners,_=_tree(wood);pixels=[]
  for x,y in next(v['coordinates']for v in inventory if v['mask']==number):
   hit=positive=0;zs=[]
   for dy in[.1,.3,.5,.7,.9]:
    for dx in[.1,.3,.5,.7,.9]:
     p,n,i,d=tree.ray_cast(Vector((x+dx,-(y+dy)/SIN,0))+RAY*6000,-RAY)
     if p is not None:hit+=1;zs.append(p.z);positive+=int(p.z>=0)
   pixels.append(dict(pixel=[x,y],wood_hits_25=hit,positive_height_wood_hits_25=positive,hit_z_range=[min(zs),max(zs)]if zs else None))
  result.append(dict(asset_id=asset,mask=number,source=str(source),source_sha256=sha(source),pixels=pixels))
 write_json(ROOT/'source-role132-v1/subpixel-wood.json',dict(records=result,scope='Sampled5x5 pixel-center neighborhood against approved wood only; crown/foliage excluded. Height compared with Z0. No semantic role proof or exhaustive area convergence claim. No model edits.'));print([{r['mask']:sum(v['positive_height_wood_hits_25']>0 for v in r['pixels'])}for r in result],flush=True)
finally:release()
