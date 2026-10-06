"""Restrict one texture retry to existing unknown physical leaf surfaces."""
import sys,json,hashlib,shutil,collections
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';out=e/'foliage-detail-retry-v1';assert not out.exists();acquire()
 model=e/'baked-preserved-v5/worker.blend';assert sha(model)=='324c3f747958806798d729f1f1a4bcc37a022bf95d5e7e580a6e6916b918689f';bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');mesh=obj.data;mesh.calc_loop_triangles();verts=[obj.matrix_world@v.co for v in mesh.vertices];tris=list(mesh.loop_triangles);bvh=BVHTree.FromPolygons(verts,[list(t.vertices) for t in tris],all_triangles=True);atlas={};source=mesh.color_attributes['Source ownership']
 for slot,m in enumerate(mesh.materials):
  if not m or not m.get('foliage_physical_opacity'):continue
  node=next(n for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);a=np.empty(len(node.image.pixels),np.float32);node.image.pixels.foreach_get(a);atlas[slot]=(a.reshape(node.image.size[1],node.image.size[0],4),mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map])
 def unknown_leaf(origin,direction):
  for _ in range(256):
   point,normal,index,distance=bvh.ray_cast(origin,direction)
   if point is None:return False
   tri=tris[index];face=mesh.polygons[tri.polygon_index]
   if face.material_index not in atlas:return False
   a,uv=atlas[face.material_index];mapped=barycentric_transform(point,*[verts[v] for v in tri.vertices],*[Vector((*uv.data[i].uv,0)) for i in tri.loops]);x=min(a.shape[1]-1,int((mapped.x%1)*a.shape[1]));y=min(a.shape[0]-1,int((mapped.y%1)*a.shape[0]))
   if a[y,x,3]<.5:origin=point+direction*1e-4;continue
   flags={source.data[i].color[0] for i in face.loop_indices};assert flags in ({0.},{1.});return flags=={0.}
  return False
 manifest=json.loads((e/'views.json').read_text());oldmask=np.array(Image.open(e/'mask.png').convert('RGBA'));editable=np.zeros(oldmask.shape[:2],bool);counts=[]
 for index,view in enumerate(manifest['views']):
  x0=(index%4)*384;y0=(index//4)*384;matrix=Matrix(view['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,-1));direction.normalize();scale=view['ortho_scale'];count=0
  yy,xx=np.where(oldmask[y0:y0+384,x0:x0+384,3]==0)
  for y,x in zip(yy,xx):
   origin=matrix@Vector((((int(x)+.5)/384-.5)*scale,(.5-(int(y)+.5)/384)*scale,0))
   if unknown_leaf(origin,direction):editable[y0+y,x0+x]=True;count+=1
  counts.append(count)
 assert editable.any() and not np.any(editable&(oldmask[...,3]!=0));out.mkdir();mask=oldmask.copy();mask[...,3]=255;mask[editable,3]=0;Image.fromarray(mask).save(out/'mask.png')
 for name in ['input.png','approval.json','views.json']:shutil.copyfile(e/name,out/name)
 refs=json.loads((e/'auxiliary-references.json').read_text());refout=[]
 for index,box in enumerate([(72,62,172,142),(75,65,175,145)]):
  ref=refs['references'][index];im=Image.open(ref['parent_image']).crop(box).resize((600,480),Image.Resampling.NEAREST);p=out/(ref['asset_id']+'-leaf-detail.png');im.save(p);refout.append(dict(source='material',file=str(p),sha256=sha(p),asset_id=ref['asset_id'],role='Fine separate leaf-scale marks and irregular small clusters only. Never imitate broad directional streaks, smooth moss, grass strands, draped fabric or large smeared patches. Keep target olive/lime palette and physical holes.',parent_image=ref['parent_image'],parent_sha256=sha(Path(ref['parent_image'])),crop=list(box),nearest_scale=6))
 lighting=e/'lighting.png'
 if not lighting.exists():
  candidates=[p for p in e.iterdir() if p.is_file() and sha(p)==refs['lighting_sha256']];assert len(candidates)==1;lighting=candidates[0]
 aux=dict(version=1,input_sha256=sha(out/'input.png'),lighting_sha256=sha(lighting),references=refout);(out/'auxiliary-references.json').write_text(json.dumps(aux,indent=2)+'\n')
 receipt=dict(status='Prepared one unknown-foliage-only retry on unchanged approved input',source_model_sha256=sha(model),input_sha256=sha(out/'input.png'),original_input_sha256=sha(e/'input.png'),mask_sha256=sha(out/'mask.png'),original_mask_sha256=sha(e/'mask.png'),editable_per_view=counts,editable_total=int(editable.sum()),source_known_edits=0,geometry_alpha_uv_changed=False,lighting=str(lighting),limitation='Conservative nearest pixel-centre physical-alpha visibility subset; boundary samples not established as unknown foliage stay protected. Only generated RGB from this subset may replace the prior material candidate; wood and other pixels must retain prior values.')
 (out/'preparation.json').write_text(json.dumps(receipt,indent=2)+'\n');release();print(json.dumps(receipt))
if __name__=='__main__':main()
