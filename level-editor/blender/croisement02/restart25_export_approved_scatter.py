"""Stage exact approved scatter surfaces as private named glTF scenes."""
import hashlib, io, json, os, shutil, struct, sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from render_slots import acquire, release
from restart25_audit_approved_state_materials import BASE, RECEIPT, EXPECTED, sha
DEST=BASE/'restart25-approved-state-materialization-v1/scatter-export-v1'
MODEL=BASE/'restart9-hiding-scatter/scatter-surfaces-v2/model.blend'
MANIFEST=MODEL.with_name('manifest.json')

def pixel_digest(data):
 im=Image.open(io.BytesIO(data)).convert('RGBA')
 return {'size':list(im.size),'rgba_sha256':hashlib.sha256(im.tobytes()).hexdigest()}

def signature(ob):
 mesh=ob.data;mesh.calc_loop_triangles()
 coords=np.array([list(ob.matrix_world@v.co) for v in mesh.vertices],dtype='<f8')
 uvs=np.array([list(u.uv) for u in mesh.uv_layers.active.data],dtype='<f4')
 return {'name':ob.name,'vertices':len(mesh.vertices),'triangles':len(mesh.loop_triangles),
         'world_vertices_sha256':hashlib.sha256(coords.tobytes()).hexdigest(),
         'uv_sha256':hashlib.sha256(uvs.tobytes()).hexdigest(),
         'matrix':[list(r) for r in ob.matrix_world],
         'materials':[m.name for m in mesh.materials]}

def main():
 assert sha(RECEIPT)==EXPECTED
 receipt=json.loads(RECEIPT.read_text())
 members=[m for c in receipt['decisions_by_card'] for m in c['members'] if m['asset_id']=='croisement02-leaf-scatter-source-surfaces'];assert len(members)==1
 approval=members[0];assert Path(approval['model'])==MODEL.resolve()
 assert sha(MODEL)==approval['model_sha256']
 manifest=json.loads(MANIFEST.read_text());assert manifest['model_sha256']==approval['model_sha256']
 mem=int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024
 assert mem>=6*1024**3 and shutil.disk_usage(BASE).free>=10*1024**3
 DEST.mkdir(parents=True,exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(MODEL));bpy.context.view_layer.update()
 names=[n for r in manifest['records'] for n in r['objects']];assert len(names)==len(set(names))==23
 objects={n:bpy.data.objects[n] for n in names};assert all(o.type=='MESH' for o in objects.values())
 before={n:signature(o) for n,o in objects.items()};images={}
 for ob in objects.values():
  for material in ob.data.materials:
   for node in material.node_tree.nodes:
    if node.type=='TEX_IMAGE' and node.image:
     assert node.image.packed_file
     images[node.image.name]=pixel_digest(bytes(node.image.packed_file.data))
 # Remove only unrelated scene objects, never modify the approved input worker.
 for ob in list(bpy.data.objects):
  if ob.name not in objects:bpy.data.objects.remove(ob,do_unlink=True)
 scenes=[]
 for record in manifest['records']:
  name=f"scatter-site-{record['index']:02}"
  group=bpy.data.objects.new(name,None);bpy.context.scene.collection.objects.link(group);group['approved_scatter_scene']=name
  for n in record['objects']:
   ob=objects[n];world=ob.matrix_world.copy();ob.parent=group;ob.matrix_world=world;ob.hide_render=False;ob.hide_set(False)
  scenes.append({'name':name,'objects':record['objects'],'instances':record['instances']})
 bpy.context.view_layer.update();assert before=={n:signature(o) for n,o in objects.items()}
 derivative=DEST/'scatter.blend';bpy.ops.wm.save_as_mainfile(filepath=str(derivative))
 bpy.ops.wm.open_mainfile(filepath=str(derivative));bpy.context.view_layer.update()
 assert before=={n:signature(bpy.data.objects[n]) for n in names}
 target=DEST/'scatter.glb'
 bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',export_extras=True,export_animations=False,export_cameras=False,export_lights=False,export_yup=True,export_apply=False)
 data=target.read_bytes();length,kind=struct.unpack_from('<II',data,12);assert kind==0x4e4f534a
 doc=json.loads(data[20:20+length]);bins=data[20+length:];assert struct.unpack_from('<I',bins,4)[0]==0x004e4942
 binary=bins[8:];actual_images=[]
 for image in doc['images']:
  view=doc['bufferViews'][image['bufferView']];start=view.get('byteOffset',0);actual_images.append(pixel_digest(binary[start:start+view['byteLength']]))
 expected_set={(tuple(i['size']),i['rgba_sha256']) for i in images.values()}
 actual_set={(tuple(i['size']),i['rgba_sha256']) for i in actual_images}
 assert actual_set==expected_set,(actual_set,expected_set)
 triangle_count=sum(doc['accessors'][p['indices']]['count']//3 for m in doc['meshes'] for p in m['primitives'])
 assert triangle_count==sum(r['triangles'] for r in before.values())
 gltf_scenes=[]
 for scene in scenes:
  ids=[i for i,n in enumerate(doc['nodes']) if n.get('extras',{}).get('approved_scatter_scene')==scene['name']];assert len(ids)==1
  gltf_scenes.append({'name':scene['name'],'nodes':ids})
 doc['scenes']=gltf_scenes;doc['scene']=0
 payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4)
 target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(bins))+struct.pack('<II',len(payload),0x4e4f534a)+payload+bins)
 assert sha(MODEL)==approval['model_sha256'] and sha(RECEIPT)==EXPECTED
 report={'status':'PRIVATE_APPROVED_SOURCE_SURFACE_EXPORT','approval':{'path':str(RECEIPT),'sha256':EXPECTED,'member':approval},
 'source_manifest':{'path':str(MANIFEST),'sha256':sha(MANIFEST)},'worker_unchanged':True,'saved_reopened_world_geometry_uv_materials_exact':True,
 'embedded_rgba_exact':True,'triangles':triangle_count,'source_images':images,'exported_images':actual_images,
 'model':str(target),'model_sha256':sha(target),'derivative':str(derivative),'derivative_sha256':sha(derivative),
 'scenes':scenes,'source_objects':list(before.values()),'instances':sum(len(s['instances']) for s in scenes),
 'absent_applied':manifest['missing_applied_instance'],
 'scope':'Approved terrain-projected final scatter only. Saved world placement; no extra instance translation. Initial mounds, loose-leaf volumes, transition meshes, current receiver integration, browser appearance verification and canonical publication are separate.'}
 (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({k:report[k] for k in ['status','model','model_sha256','triangles','instances','embedded_rgba_exact','saved_reopened_world_geometry_uv_materials_exact']}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
