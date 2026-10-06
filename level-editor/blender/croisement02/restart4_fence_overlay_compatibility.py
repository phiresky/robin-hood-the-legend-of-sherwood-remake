"""Verify the approved narrow fence overlay against current ground and paired states."""
import sys,json,struct,math,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from restart2_export_receiver_surfaces import clip_polygon,screen
from review_bank_candidate import camera
from tree_geometry import RAY,SIN,COS
D=OUT/'restart4-fence-overlay-current-ground-v1'
GROUND=OUT/'restart4-final-floor-bake-v1/model.blend';GLB=OUT/'restart2-state/cleared-fence-receiver-export-v2/model.glb';PATCH=OUT/'source-states/mission-patches/mission-Emb05_FoB_MP-patch-022/transition-000.png'
INITIAL=OUT/'restart3-initial-fence/geometry-v6/model.blend';APPLIED=OUT/'restart2-textures/approved-cleared-fence-fill-v2/croisement02-south-field-wattle-fence-cleared-state/experiment/bake-v1/worker.blend'
def main():
 assert not D.exists();pins={GROUND:'206c7c562b9aa9fc204ce42c0b0e9b1b78a22bee79ac00e6dab7fbbc59c64630',GLB:'29db0d17df1c600766b3590c745adb50d4a74366ba32a2e99d97148d520be2c1',PATCH:'ec3b919539c3aa8fdfa4a43399479c48644a371c494bf0146128a20027fb2170',INITIAL:'46579f5398d1495b13e8d8433fa1a0687be447e025cd9ba745f5d7251076c5a0',APPLIED:'13978cf8bffdf7f3ef5deff863f11ea40c008bbf01b3f0d23f201d937203ea82'};assert all(sha(p)==h for p,h in pins.items());D.mkdir()
 data=GLB.read_bytes();n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n]);binary=data[28+n:];assert len(doc['images'])==1 and len(doc['meshes'])==1 and 'animations'not in doc
 imageview=doc['bufferViews'][doc['images'][0]['bufferView']];png=binary[imageview.get('byteOffset',0):imageview.get('byteOffset',0)+imageview['byteLength']];assert hashlib.sha256(png).hexdigest()==sha(PATCH)
 def accessor(index,width):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];return np.frombuffer(binary,dtype='<f4',count=a['count']*width,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(-1,width)
 primitive=doc['meshes'][0]['primitives'][0];positions=accessor(primitive['attributes']['POSITION'],3);uv=accessor(primitive['attributes']['TEXCOORD_0'],2)
 bpy.ops.wm.open_mainfile(filepath=str(GROUND));bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain'];sig=geometry(ground);node,original=atlas(ground);assert sig=='d985f5556fd548f88d9d2d0aa9bf9c6e98bb9ef419cbd159afbf73496b7dd934';ground.data.calc_loop_triangles();points=np.array([ground.matrix_world@v.co for v in ground.data.vertices]);verts=[];tex=[]
 for tri in ground.data.loop_triangles:
  poly=[points[i]for i in tri.vertices]
  for axis,bound,keep in [(0,1018,1),(0,1170,-1),(1,811,1),(1,963,-1)]:
   poly=clip_polygon(poly,axis,bound,keep)
   if not poly:break
  for i in range(1,len(poly)-1):
   tri=[poly[0],poly[i],poly[i+1]]
   if np.linalg.norm(np.cross(tri[1]-tri[0],tri[2]-tri[0]))<1e-8:continue
   for p in tri:
    sx,sy=screen(p);verts.append(p+np.array(RAY)*.002);tex.append([(sx-1018)/152,(sy-811)/152])
 verts=np.array(verts);expected=np.column_stack([verts[:,0],verts[:,2],-verts[:,1]]).astype('<f4');assert np.array_equal(expected,positions) and np.array_equal(np.array(tex,dtype='<f4'),uv)
 scene=bpy.data.scenes.new('Current floor and approved fence states');bpy.context.window.scene=scene;link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update()
 prior=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(GLB));overlay=[o for o in bpy.data.objects if o not in prior and o.type=='MESH'];assert len(overlay)==1
 for o in overlay:link(scene,o)
 imported={};applied_names=json.loads((OUT/'fence-state-candidate-v2/applied-views.json').read_text())['object_names']
 for label,path in [('initial',INITIAL),('applied',APPLIED)]:
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.objects=list(src.objects)if label=='initial'else applied_names
  chosen=[o for o in dst.objects if o and o.type=='MESH' and (label=='applied' or o.get('asset_group')=='croisement02-south-field-wattle-fence')];assert chosen
  for o in chosen:link(scene,o)
  imported[label]=chosen
 bpy.context.view_layer.update();signatures={o.name:geometry(o)for os in imported.values()for o in os};target=Vector((1094,-887/SIN,18));records=[]
 for i in range(8):
  a=i*math.pi/4;direction=Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN))
  for state in ['initial','applied']:
   for label,objs in imported.items():
    for o in objs:o.hide_render=label!=state
   for o in overlay:o.hide_render=state!='applied'
   camera(scene,target,direction,640,480,330);scene.render.filepath=str(D/f'{state}-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name);records.append(dict(state=state,view=i,file=f'{state}-{i}.png',sha256=sha(D/f'{state}-{i}.png')))
 sheet=Image.new('RGB',(1280,8*508),'#292929');draw=ImageDraw.Draw(sheet)
 for i in range(8):
  for j,state in enumerate(['initial','applied']):sheet.paste(Image.open(D/f'{state}-{i}.png'),(j*640,i*508+28));draw.text((j*640+6,i*508+7),('Original camera'if i==0 else f'Orbit{i}')+' '+state,fill='white')
 sheet.save(D/'paired8.png');assert geometry(ground)==sig and np.array_equal(atlas(ground)[1],original);assert signatures=={o.name:geometry(o)for os in imported.values()for o in os};assert all(sha(p)==h for p,h in pins.items())
 write_json(D/'validation.json',dict(status='PASS exact reused overlay compatibility; visual review pending',ground_model_sha256=sha(GROUND),overlay_glb_sha256=sha(GLB),source_patch_sha256=sha(PATCH),source_patch_pixels=23104,ground_geometry_signature=sig,overlay_vertices_float32_exact_current_clipping=True,overlay_uv_float32_exact_current_clipping=True,overlay_embedded_png_exact=True,depth_offset_ray=0.002,outside_overlay_geometry=False,ground_entire_rgba_unchanged=True,new_model_or_glb_written=False,source_pins={str(p):h for p,h in pins.items()},views=records,paired_sheet_sha256=sha(D/'paired8.png'),limits=['Applied-state overlay only; initial retains exact current ground without terminal art.','Current206c base ground appearance is pending user approval; this report is not publication authority.','Initial diagnostic uses original-world465 geometry worker; runtime rebase is a separate byte-equivalent integration proof.','No source assignment, new geometry, texture synthesis or full-ground replacement occurred.']))
 print('COMPATIBILITY COMPLETE',flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
