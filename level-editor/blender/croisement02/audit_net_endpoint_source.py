"""Measure reopened net endpoint source coverage and per-part native RGB protection."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from log_trap_state_candidate import sha,point
from tree_geometry import RAY


def main():
 base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve() if '--candidate' in sys.argv else OUT/'net-endpoint-candidate-v2';manifest=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==manifest['model_sha256'];fit=json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text());row=next(r for r in fit['records']if r['patch_id']==manifest['source_patch']);source=np.array(Image.open(row['source']).convert('RGBA'));x,y,w,h=row['bbox'];bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));objects=[];records=[]
 for record in manifest['objects']:
  obj=bpy.data.objects[record['object']];vertices=[obj.matrix_world@v.co for v in obj.data.vertices];tree=BVHTree.FromPolygons(vertices,[list(p.vertices)for p in obj.data.polygons]);path=base/(obj.name.replace(' ','-')+'-observed.png');pixels=np.array(Image.open(path).convert('RGBA'));known=pixels[:,:,3]>0;assert np.array_equal(source[:,:,:3][known],pixels[:,:,:3][known]);bm=bmesh.new();bm.from_mesh(obj.data);closed=all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);bm.free();assert closed and volume>0;objects.append((obj,tree,pixels));records.append(dict(object=obj.name,closed=closed,volume=volume,known_source_rgb_unchanged=True,accepted=0,gray_first_hits=0))
 counts=dict(opaque_source=int((source[:,:,3]>0).sum()),accepted_source=0,unknown_gray_geometry=0,no_geometry=0);overlay=source.copy();missing=[]
 for py,px in zip(*np.where(source[:,:,3]>0)):
  origin=point(x+float(px)+.5,y+float(py)+.5,0)+RAY*5000;hits=[]
  for index,(obj,tree,pixels)in enumerate(objects):
   location,normal,face,distance=tree.ray_cast(origin,-RAY)
   if location is not None:hits.append((distance,index,face))
  if not hits:counts['no_geometry']+=1;overlay[py,px]=[255,40,220,255];missing.append([int(px),int(py)]);continue
  distance,index,face=min(hits);obj,tree,pixels=objects[index]
  if obj.data.polygons[face].material_index==0 and pixels[py,px,3]>0:counts['accepted_source']+=1;records[index]['accepted']+=1
  else:counts['unknown_gray_geometry']+=1;records[index]['gray_first_hits']+=1;overlay[py,px]=[255,150,0,255]
 result=dict(status='Source diagnostic; manual part ownership and attachments remain provisional',model_sha256=manifest['model_sha256'],source_sha256=sha(Path(row['source'])),counts=counts,objects=records,missing_source_crop_coordinates=missing,limitations=['Underlying source RGB is checked exactly; rendered lighting is separate.','A first-hit geometric receiver does not establish the correctness of an inferred part partition.','Attachment support and full mission timing are not proven by this audit.']);(base/'source-audit.json').write_text(json.dumps(result,indent=2)+'\n');Image.fromarray(overlay).resize((w*6,h*6),Image.Resampling.NEAREST).save(base/'source-audit.png');print(json.dumps(counts))
if __name__=='__main__':main()
