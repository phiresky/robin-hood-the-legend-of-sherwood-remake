"""Read-only leaf UV stretch and atlas inspection before further generation."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release

def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';out=e/'leaf-mapping-audit-v1';out.mkdir(exist_ok=False);model=e/'baked-preserved-v5/worker.blend';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();assert sha(model)=='324c3f747958806798d729f1f1a4bcc37a022bf95d5e7e580a6e6916b918689f';acquire();bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');mesh=obj.data;mesh.calc_loop_triangles();ownership=mesh.color_attributes['Source ownership'];rows=[];panels=[]
 for slot,mat in enumerate(mesh.materials):
  if not mat or not mat.get('foliage_physical_opacity'):continue
  node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);uv=mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map];width,height=node.image.size;ratios=[];areas=[];flags=set()
  for tri in mesh.loop_triangles:
   face=mesh.polygons[tri.polygon_index]
   if face.material_index!=slot:continue
   flags.update(ownership.data[i].color[0] for i in face.loop_indices);p=np.array([obj.matrix_world@mesh.vertices[i].co for i in tri.vertices]);t=np.array([uv.data[i].uv[:] for i in tri.loops])*[width,height];d=np.stack([t[1]-t[0],t[2]-t[0]],axis=1)
   if abs(np.linalg.det(d))<1e-10:continue
   jac=np.stack([p[1]-p[0],p[2]-p[0]],axis=1)@np.linalg.inv(d);singular=np.linalg.svd(jac,compute_uv=False)
   if singular[-1]<=1e-12:continue
   ratios.append(float(singular[0]/singular[-1]));areas.append(float(np.linalg.norm(np.cross(p[1]-p[0],p[2]-p[0]))*.5))
  a=np.asarray(ratios);w=np.asarray(areas);assert len(a);row=dict(slot=slot,material=mat.name,source_ownership=sorted(flags),atlas_dimensions=[width,height],triangles=len(a),uv_world_anisotropy_median=float(np.median(a)),uv_world_anisotropy_p95=float(np.quantile(a,.95)),uv_world_anisotropy_max=float(a.max()),surface_area_over_ratio2=float(w[a>2].sum()/w.sum()),surface_area_over_ratio4=float(w[a>4].sum()/w.sum()));rows.append(row)
  if slot in (17,18,19,20):
   pix=np.empty(len(node.image.pixels),np.float32);node.image.pixels.foreach_get(pix);pix=pix.reshape(height,width,4);rgb=pix[...,:3];srgb=np.where(rgb<=.0031308,12.92*rgb,1.055*np.power(np.maximum(rgb,0),1/2.4)-.055);rgba=np.concatenate([np.clip(srgb,0,1),pix[...,3:4]],axis=2);im=Image.fromarray(np.rint(rgba[::-1]*255).astype('uint8'));im.save(out/f'slot-{slot}-atlas.png');panels.append((slot,mat.name,im))
 sheet=Image.new('RGB',(1200,700),(32,32,32));draw=ImageDraw.Draw(sheet)
 for i,(slot,name,im) in enumerate(panels):
  scale=min(580//im.width,310//im.height);scale=max(1,scale);scaled=im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST);x=(i%2)*600;y=(i//2)*350;sheet.paste(scaled,(x+5,y+35),scaled);draw.text((x+5,y+8),f'Slot {slot}: {name}',fill='white')
 sheet.save(out/'atlas-closeups.png');receipt=dict(model_sha256=sha(model),rows=rows,method='Triangle world-space surface Jacobian with respect to physical atlas texel coordinates. Singular-value ratio measures mapping anisotropy independent of camera projection; area fractions weighted by actual triangle world area.',limitations=['Anisotropy is not proof of texture generation failure by itself; camera foreshortening is separate.','No geometry, UV, image, alpha, source ownership or model file changed. Atlas images are diagnostic sRGB nearest enlargements.']);(out/'mapping.json').write_text(json.dumps(receipt,indent=2)+'\n');assert sha(model)==receipt['model_sha256'];release();print(json.dumps(rows))
if __name__=='__main__':main()
