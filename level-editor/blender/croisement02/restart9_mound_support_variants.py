"""Place shallow leaf volumes on exact terrain along preserved source rays."""
from pathlib import Path
import sys,json,hashlib
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=OUT/'restart9-hiding-scatter';out=root/'mound-support-variants-v1';out.mkdir(exist_ok=False);audit=json.loads((root/'terrain-receivers-v2/report.json').read_text());vertices=[];triangles=[]
 for pin in audit['models']:
  path=Path(pin['path']);assert sha(path)==pin['sha256'];bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
  for row in pin['objects']:
   o=bpy.data.objects[row['name']];assert np.max(abs(np.array(o.matrix_world)-np.array(row['matrix_world'])))<1e-8;o.data.calc_loop_triangles();start=len(vertices);vertices.extend([o.matrix_world@v.co for v in o.data.vertices]);triangles.extend([tuple(start+i for i in t.vertices)for t in o.data.loop_triangles])
 tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);model=root/'mound-flat-v2/model.blend';assert sha(model)=='8fec7a325c268e14613424cd865af225247f2effa3ea3fceb2a01b0c14e6868e';bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;source=bpy.data.objects['Hiding cover initial leaf mound'];points=[v.co.copy()for v in source.data.vertices];groups={};records=[]
 for r in audit['records']:
  if not r['profile'].endswith('hiding Pc'):continue
  key=tuple(r['display_position']);groups.setdefault(key,dict(record=r,instances=[]))['instances'].append(r['id'])
 for index,(position,g)in enumerate(groups.items()):
  record=g['record'];offset=record['initial']['offset'];cx=position[0]+offset[0]+26;cy=position[1]+offset[1]+16.5;o=source.copy();o.data=source.data.copy();o.name=f'Hiding mound {index:02}';scene.collection.objects.link(o);heights=[];errors=[]
  for v,p in zip(o.data.vertices,points):
   sx=p.x+cx;sy=-p.y*SIN-p.z*COS+cy;hit,normal,ti,d=tree.ray_cast(Vector((sx,-sy/SIN,0))+RAY*6000,-RAY);assert hit is not None;v.co=hit+RAY*(p.z/RAY.z);heights.append(float(hit.z));errors.append(max(abs(v.co.x-sx),abs(-v.co.y*SIN-v.co.z*COS-sy)))
  o.data.update();bm=bmesh.new();bm.from_mesh(o.data);closed=all(e.is_manifold for e in bm.edges)and all(v.is_manifold for v in bm.verts);volume=bm.calc_volume(signed=True);bm.free();assert closed and volume>0 and max(errors)<.001
  o['source_instances']=json.dumps(g['instances']);records.append(dict(object=o.name,instances=g['instances'],display_position=position,source_sha256=record['initial']['sha256'],support_height_range=[min(heights),max(heights)],closed=closed,volume=volume,maximum_projection_error=max(errors)))
 bpy.data.objects.remove(source,do_unlink=True);path=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(path));(out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_CONTACT_VARIANTS',model_sha256=sha(path),source_model_sha256=sha(model),terrain_models=audit['models'],records=records,instance_count=sum(len(r['instances'])for r in records),scope='Support sampled at every existing vertex along fixed native rays. No source pixel or UV change. Terrain-only contact; foreground and interior support sampling remain pending.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
