"""Export a private multi-scene receiver derivative with unchanged source materials."""
import sys,json,hashlib,struct
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from render_slots import acquire,release
ROOT=HERE.parents[1]/'work/croisement02-refinement/restart10-hole-aperture'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def root(name):
 ob=bpy.data.objects.new(name,None);bpy.context.scene.collection.objects.link(ob);ob['aperture_scene']=name;return ob

def main():
 dest=ROOT/'export-v1';dest.mkdir(exist_ok=False,parents=True)
 packet_path=ROOT/'packet-v1/packet.json';assert sha(packet_path)=='8e2c6dfdda71b556da620d790431b1d7406dcc05a0d4d68729e5299fed5d4a8f'
 packet=json.loads(packet_path.read_text());bpy.ops.wm.read_factory_settings(use_empty=True)
 baseline=root('original-receivers');derivative=root('aperture-receivers');sources={};proof=[]
 for parent in packet['parents']:
  names=[m['name']for m in packet['meshes']if m['parent']==parent['sha256']]
  if not names:continue
  assert sha(parent['path'])==parent['sha256']
  with bpy.data.libraries.load(parent['path'],link=False)as(data,loaded):loaded.objects=names
  for ob in loaded.objects:bpy.context.scene.collection.objects.link(ob)
  bpy.context.view_layer.update()
  for ob in loaded.objects:
   expected=next(r for r in parent['objects']if r['name']==ob.name)['matrix'];assert np.max(np.abs(np.array(ob.matrix_world)-expected))<1e-7
   world=ob.matrix_world.copy();ob.parent=baseline;ob.matrix_world=world;ob.hide_render=False;ob.hide_set(False);sources[ob.name]=ob
  proof.append({'path':parent['path'],'sha256':parent['sha256']})
 bpy.context.view_layer.update();part_rows=[]
 for record in packet['meshes']:
  original=sources[record['name']];original.data.calc_loop_triangles();triangles=list(original.data.loop_triangles)
  nmatrix=original.matrix_world.to_3x3().inverted().transposed()
  for tag,rows in record['groups'].items():
   if not rows:continue
   name=record['name']+' / '+tag
   if len(record['groups'])==1:
    ob=original.copy();ob.data=original.data;bpy.context.scene.collection.objects.link(ob);world=ob.matrix_world.copy();ob.parent=derivative;ob.matrix_world=world;ob.name=name
   else:
    verts=[];faces=[];uvs={k:[]for k in record['source'][0]['uv']};normals=[];indices=[]
    for row in rows:
     ids=list(range(len(verts),len(verts)+3));verts.extend(row['xyz']);faces.append(ids);indices.append(row['material'])
     tri=triangles[row['source_triangle']];base=[nmatrix@original.data.corner_normals[i].vector for i in tri.loops]
     weights=row.get('weights',np.eye(3).tolist())
     for w in weights:normals.append(tuple(sum((base[i]*w[i]for i in range(3)),Vector()).normalized()))
     for k,v in row['uv'].items():uvs[k].extend(v)
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update();ob=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(ob);ob.parent=derivative
    for mat in original.data.materials:mesh.materials.append(mat)
    for poly,mi in zip(mesh.polygons,indices):poly.material_index=mi;poly.use_smooth=True
    for k,values in uvs.items():
     layer=mesh.uv_layers.new(name=k)
     for loop in mesh.loops:layer.data[loop.index].uv=values[loop.vertex_index]
    mesh.normals_split_custom_set(normals)
   ob['aperture_source_object']=record['name'];ob['aperture_component']=tag
   if tag!='outside':ob['reveal_hide_when_applied']=next(h['triggers']for h in packet['holes']if h['id']==tag)
   part_rows.append({'name':ob.name,'source':record['name'],'component':tag,'triangles':len(rows),'materials_same_datablocks':all(a is b for a,b in zip(ob.data.materials,original.data.materials))})
 for phase,record in packet['endpoints'].items():
  assert sha(record['path'])==record['sha256'];group=root('endpoint-'+phase)
  with bpy.data.libraries.load(record['path'],link=False)as(data,loaded):loaded.objects=data.objects
  for ob in loaded.objects:
   if ob.type!='MESH':continue
   bpy.context.scene.collection.objects.link(ob);ob.parent=group;ob.hide_render=False;ob.hide_set(False)
  proof.append({'path':record['path'],'sha256':record['sha256']})
 bpy.context.view_layer.update()
 target=dest/'receivers-and-endpoints.glb'
 bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',export_extras=True,export_animations=False,export_cameras=False,export_lights=False,export_yup=True,export_apply=False)
 data=target.read_bytes();length,kind=struct.unpack_from('<II',data,12);assert kind==0x4e4f534a;doc=json.loads(data[20:20+length]);bins=data[20+length:]
 scenes=[]
 for name in ['original-receivers','aperture-receivers','endpoint-initial','endpoint-applied']:
  ids=[i for i,n in enumerate(doc['nodes'])if n.get('extras',{}).get('aperture_scene')==name];assert len(ids)==1,(name,ids);scenes.append({'name':name,'nodes':ids})
 doc['scenes']=scenes;doc['scene']=0
 payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4);target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(bins))+struct.pack('<II',len(payload),0x4e4f534a)+payload+bins)
 for pin in proof:assert sha(pin['path'])==pin['sha256']
 report={'model':str(target),'model_sha256':sha(target),'packet_sha256':sha(packet_path),'scenes':scenes,'source_pins':proof,'parts':part_rows,'material_datablocks_exact':all(r['materials_same_datablocks']for r in part_rows),'images_shared_between_scenes':len(doc.get('images',[])),'scope':'Private export only; default scene original receivers. Applied caps require production preview controller; no library/catalog mutation.'}
 (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
