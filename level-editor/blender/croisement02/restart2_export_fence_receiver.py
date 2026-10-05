"""Derive an isolated applied fence patch on its approved physical ground receiver."""
import sys,json
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import RAY
from restart2_export_receiver_surfaces import GLB,screen,clip_polygon
DEST=OUT/'restart2-state/cleared-fence-receiver-export-v2';BASE=OUT/'restart3-fence-receiver/terminal-v3'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 assert sha(BASE/'model.blend')=='95e03efae7f3a93725ecf953c43e473eca1ed71e0c4885be81f953b94a90bce3'
 approval=BASE/'user-state-application-approval.json';assert sha(approval)=='8552bff03d25d3bc30ffc221c3d38716ddc29ff3b94b538f1af166a359a321d0';source=OUT/'source-states/mission-patches/mission-Emb05_FoB_MP-patch-022/transition-000.png';assert sha(source)=='ec3b919539c3aa8fdfa4a43399479c48644a371c494bf0146128a20027fb2170';acquire()
 try:
  DEST.mkdir();bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain'];ground.data.calc_loop_triangles();points=np.array([ground.matrix_world@v.co for v in ground.data.vertices]);bounds=json.loads((BASE/'validation.json').read_text())['receiver_world_z_range'];assert np.max(abs(np.array([points[:,2].min(),points[:,2].max()])-np.array(bounds)))<1e-8;verts=[];uv=[];x,y,w,h=1018,811,152,152
  for triangle in ground.data.loop_triangles:
   poly=[points[i]for i in triangle.vertices]
   for axis,bound,keep in [(0,x,1),(0,x+w,-1),(1,y,1),(1,y+h,-1)]:
    poly=clip_polygon(poly,axis,bound,keep)
    if not poly:break
   for i in range(1,len(poly)-1):
    tri=[poly[0],poly[i],poly[i+1]]
    if np.linalg.norm(np.cross(tri[1]-tri[0],tri[2]-tri[0]))<1e-8:continue
    for p in tri:
     sx,sy=screen(p);verts.append(p+np.array(RAY)*.002);uv.append([(sx-x)/w,(sy-y)/h])
  assert verts;g=GLB();g.layer('Cleared fence terminal ground',verts,uv,source.read_bytes(),{'native_state':'applied','native_frame':0,'source_sha256':sha(source)},True)
  # Static endpoint: strip empty animation metadata after generic serialization.
  file=DEST/'model.glb';g.save(file)
  import struct
  data=file.read_bytes();length=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+length]);del doc['animations'];binary=data[28+length:];encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);file.write_bytes(struct.pack('<III',0x46546c67,2,28+len(encoded)+len(binary))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(binary),0x004e4942)+binary)
  write_json(DEST/'manifest.json',{'status':'Approved source-art state application export; browser proof pending','model_sha256':sha(BASE/'model.blend'),'approval_sha256':sha(approval),'glb_sha256':sha(file),'source':str(source),'source_sha256':sha(source),'bbox':[x,y,w,h],'triangles':len(verts)//3,'receiver':'Croisement02 Terrain','nominal_world_z':0,'actual_world_z_range':bounds,'source_preserving_ray_offset':.002,'limits':['Applied-state overlay only; base ground unchanged.','Activation must be coordinated with cleared fence body and mission state.','No generated texture or motion claim.']})
 finally:release()
if __name__=='__main__':main()
