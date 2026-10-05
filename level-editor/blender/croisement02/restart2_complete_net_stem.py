"""Complete the observed upper rope neck without moving the branch attachment."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from log_trap_state_candidate import material
BASE=OUT/'restart2-state/net-empty01-v7';DEST=OUT/'restart2-state/net-empty01-v8'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  DEST.mkdir();prior=json.loads((BASE/'report.json').read_text());assert sha(BASE/'model.blend')==prior['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;row=next(r for r in json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text())['records']if r['family']=='piege01'and r['variant']=='e');x,y,w,h=row['bbox'];world_y=-1107/SIN+RAY.y*54;vertices=[];faces=[];rings=[(.5,35.3,.35),(2,35.2,1.65),(4.5,35.3,2.0),(7.5,35.7,2.15),(10.5,35.9,1.4)]
  for sy,sx,r in rings:
   for j in range(16):
    a=j*math.pi/8;vertices.append((x+sx+r*math.cos(a),world_y+r*.65*math.sin(a),(-(y+sy)-world_y*SIN)/COS))
  for k in range(len(rings)-1):
   for j in range(16):a=k*16+j;b=k*16+(j+1)%16;faces.append((a,b,b+16,a+16))
  faces.extend([tuple(range(15,-1,-1)),tuple((len(rings)-1)*16+j for j in range(16))]);mesh=bpy.data.meshes.new('Observed upper rope neck');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new('Upper attachment knot',mesh);scene.collection.objects.link(obj)
  raw=np.array(Image.open(row['source']).convert('RGBA'));known=np.zeros((h,w),bool);known[:13,32:40]=True;raw[:,:,3]=np.where(known,raw[:,:,3],0);image=DEST/'Upper-attachment-knot-observed.png';Image.fromarray(raw).save(image);mesh.materials.append(material(image));mesh.materials.append(bpy.data.materials['Unobserved net surfaces']);uv=mesh.uv_layers.new(name='Native target projection')
  for face in mesh.polygons:
   face.material_index=0 if face.normal.dot(RAY)>.05 else 1
   for loop in face.loop_indices:
    p=mesh.vertices[mesh.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)
  # Keep the exact prior phase-source images available to the independent ray audit.
  import shutil
  for path in BASE.glob('*observed.png'):shutil.copyfile(path,DEST/path.name)
  bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'));write_json(DEST/'report.json',{**prior,'status':'Private observed rope-neck addition; reopened/native/joint checks pending','model_sha256':sha(DEST/'model.blend'),'parent_model_sha256':prior['model_sha256'],'rope_neck':{'closed_volume':volume,'source_crop_domain':[32,0,40,13],'ring_survey':rings,'depth_to_width_radius':.65},'renders':[]})
 finally:release()
if __name__=='__main__':main()
