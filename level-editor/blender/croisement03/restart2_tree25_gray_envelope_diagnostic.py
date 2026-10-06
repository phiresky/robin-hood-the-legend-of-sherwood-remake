"""Read-only pixel-centre alpha/ownership probe for saved tree review closeups."""
import sys,json,hashlib,collections
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release

def pixels(image):
 a=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(a);return a.reshape(image.size[1],image.size[0],4)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';out=e/'v5-closeup-diagnostic';dest=out/'gray-rays-envelope.json';assert not dest.exists();acquire()
 bpy.ops.wm.open_mainfile(filepath=str(e/'material-partition-v1/model.blend'));asset='croisement03-tree-25';native={}
 for o in bpy.data.objects:
  if o.type=='MESH' and o.get('asset_group')==asset:
   for slot,m in enumerate(o.data.materials):
    if m and m.get('foliage_physical_opacity'):native[slot]=pixels(next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image))
 model=e/'baked-preserved-v5/worker.blend';assert sha(model)=='324c3f747958806798d729f1f1a4bcc37a022bf95d5e7e580a6e6916b918689f';bpy.ops.wm.open_mainfile(filepath=str(model))
 obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==asset);mesh=obj.data;mesh.calc_loop_triangles();verts=[obj.matrix_world@v.co for v in mesh.vertices];tris=list(mesh.loop_triangles);bvh=BVHTree.FromPolygons(verts,[list(t.vertices) for t in tris],all_triangles=True);atlas={};source=mesh.color_attributes['Source ownership']
 for slot,m in enumerate(mesh.materials):
  if not m or not m.get('foliage_physical_opacity'):continue
  node=next(n for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);uvname=node.inputs['Vector'].links[0].from_node.uv_map
  atlas[slot]=(pixels(node.image),mesh.uv_layers[uvname],node.extension,node.interpolation)
 def trace(origin,direction):
  skipped=0
  for step in range(256):
   point,normal,tri_id,distance=bvh.ray_cast(origin,direction)
   if point is None:return 'transparent_background',skipped,None
   tri=tris[tri_id];face=mesh.polygons[tri.polygon_index];slot=face.material_index
   if slot not in atlas:return 'wood_or_other_material',skipped,None
   a,uv,extension,interpolation=atlas[slot];coords=[Vector((*uv.data[i].uv,0)) for i in tri.loops];mapped=barycentric_transform(point,*[verts[v] for v in tri.vertices],*coords);u,v=mapped.x,mapped.y
   if extension=='REPEAT':u%=1;v%=1
   x=min(a.shape[1]-1,max(0,int(u*a.shape[1])));y=min(a.shape[0]-1,max(0,int(v*a.shape[0])));color=a[y,x]
   if color[3]<.5:origin=point+direction*1e-4;skipped+=1;continue
   flags={source.data[i].color[0] for i in face.loop_indices};assert flags in ({0.},{1.})
   if flags=={1.}:return 'protected_source_leaf',skipped,slot
   base=native[slot][y,x];unchanged=np.array_equal(color,base);neutral=float(np.ptp(base[:3]))<1e-6
   return ('untouched_neutral_leaf' if unchanged and neutral else 'generated_or_colored_leaf'),skipped,slot
  return 'ray_depth_limit',skipped,None
 manifest=json.loads((e/'views-grid8-v4.json').read_text());regions=[(0,(85,95,185,195)),(1,(130,125,230,225)),(4,(130,190,270,315)),(5,(215,180,325,315))];reports=[]
 for index,box in regions:
  im=np.array(Image.open(e/f'baked-preserved-v5/actual/view-{index}-textured.png').convert('RGB'));view=manifest['views'][index];matrix=Matrix(view['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,-1));direction.normalize();scale=view['ortho_scale'];counts=collections.Counter();slots=collections.Counter();mask=Image.new('RGB',(384,384));allcounts=collections.Counter();graycount=0
  colored=(im.max(2).astype(int)-im.min(2).astype(int))>4
  points=[]
  for yy in range(384):
   xx=np.flatnonzero(colored[yy])
   if len(xx):points.extend([(int(xx[0]),yy),(int(xx[-1]),yy)])
  points=sorted(set(points))
  def cross(o,a,b):return (a[0]-o[0])*(b[1]-o[1])-(a[1]-o[1])*(b[0]-o[0])
  lower=[];upper=[]
  for p in points:
   while len(lower)>=2 and cross(lower[-2],lower[-1],p)<=0:lower.pop()
   lower.append(p)
  for p in reversed(points):
   while len(upper)>=2 and cross(upper[-2],upper[-1],p)<=0:upper.pop()
   upper.append(p)
  envelope=Image.new('L',(384,384));ImageDraw.Draw(envelope).polygon(lower[:-1]+upper[:-1],fill=255)
  palette={'transparent_background':(40,110,255),'untouched_neutral_leaf':(255,40,40),'generated_or_colored_leaf':(60,200,80),'protected_source_leaf':(255,200,20),'wood_or_other_material':(190,80,200),'ray_depth_limit':(255,255,255)}
  for y in range(box[1],box[3]):
   for x in range(box[0],box[2]):
    rgb=im[y,x].astype(int);neutral=rgb.max()-rgb.min()<=4 and rgb.max()>20
    if not neutral or not envelope.getpixel((x,y)):continue
    origin=matrix@Vector((((x+.5)/384-.5)*scale,(.5-(y+.5)/384)*scale,0));kind,skipped,slot=trace(origin,direction);counts[kind]+=1;graycount+=1;mask.putpixel((x,y),palette[kind]);
    if slot is not None:slots[str(slot)]+=1
  mask.save(out/f'gray-envelope-view-{index}.png');reports.append(dict(view=index,box=list(box),near_neutral_pixels=graycount,classifications=dict(counts),material_slots=dict(slots)))
 receipt=dict(model_sha256=sha(model),reports=reports,method='Saved render pixels with RGB spread<=4 and brightness>20, in declared closeup rectangles and inside the convex hull of colored tree pixels (excludes exterior rectangle corners). Pixel-centre BVH rays skip physical-alpha<0.5 texels, then classify exact remaining source/unknown texels against normalized approved atlas.',limitations=['Nearest texel, pixel-centre diagnostic; saved Cycles image uses antialiasing and texture filtering, so boundary pixels may differ.','No rendered alpha pass exists: saved review has opaque gray world. Background classification is geometric/physical-alpha inference, not a direct saved alpha measurement.','Rectangles are diagnostic samples, not entire-crown visibility counts.'],material_interpolation={str(k):v[3] for k,v in atlas.items()},legend=palette)
 dest.write_text(json.dumps(receipt,indent=2)+'\n');release();print(json.dumps(receipt))
if __name__=='__main__':main()
