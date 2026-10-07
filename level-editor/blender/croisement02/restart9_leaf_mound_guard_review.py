"""Reopen a leaf-cover candidate for independent pixel rays and lit geometry review."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart9-hiding-scatter/mound-flat-v2';out=worker/'independent-guard-v1';out.mkdir(exist_ok=False);rec=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();obj=bpy.data.objects['Hiding cover initial leaf mound'];tree,owners,_=_tree([obj]);obj.data.calc_loop_triangles();a=np.asarray(Image.open(rec['source']).convert('RGBA'));h,w=a.shape[:2];miss=[];foreign=[];uv_error=0.;covered=0
 for y in range(h):
  for x in range(w):
   sx=x+.5-w/2;sy=y+.5-h/2;hit,normal,index,d=tree.ray_cast(Vector((sx,-sy/SIN,0))+RAY*100,-RAY)
   if a[y,x,3]>0:
    if hit is None:miss.append([x,y]);continue
    if obj.data.loop_triangles[index].material_index!=0:foreign.append([x,y]);continue
    uv_error=max(uv_error,abs(hit.x+w/2-(x+.5)),abs(-hit.y*SIN-hit.z*COS+h/2-(y+.5)));covered+=1
   elif hit is not None:foreign.append([x,y])
 assert not miss and not foreign and uv_error<.0001,(miss,foreign,uv_error)
 scene=bpy.context.scene;light=bpy.data.objects.new('Geometry review key',bpy.data.lights.new('Geometry review key','AREA'));scene.collection.objects.link(light);light.location=(30,-55,85);light.rotation_euler=(-light.location).to_track_quat('-Z','Y').to_euler();light.data.energy=650;light.data.size=35;solid=bpy.data.materials.new('Lit geometry');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.5,.5,.5,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.8;scene.view_layers[0].material_override=solid;paths=[]
 for i in range(8):
  angle=i*math.pi/4;cam=frame(scene,[obj],Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.3);file=out/f'solid-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
 sheet(paths,out/'solid-eight-lit.png');report=dict(status='PASS',model_sha256=sha(model),source_sha256=sha(Path(rec['source'])),opaque_source_pixels=covered,missing_source_pixels=miss,foreign_pixel_centers=foreign,max_source_ray_coordinate_error=uv_error,source_RGBA_material='Embedded exact image on all first-hit top faces; no generated fill.',geometry_lit_views=[dict(path=p.name,sha256=sha(p))for p in paths],limits=['Flat reusable candidate only. Other scenery not imported.','Maximumheight4.788 is inferred; plateau placement and one sloped initial footprint need contact proofs.']);(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==rec['model_sha256']
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
