"""Export the approved initial wagon privately at the existing family anchor."""
import sys,json,hashlib,struct,shutil
from pathlib import Path
from array import array
import bpy
from mathutils import Vector
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2]
sys.path[:0]=[str(HERE),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_review import _tile
from evidence_io import sha,write_json
E=OUT/'restart4-south-cart-texture/approved-fill-v1/experiment';S=E/'underedge-fill-v2/worker.blend';D=OUT/'restart4-south-cart-export/private-v1';A=OUT/'restart3-review-batches/batch-v10/user-approval.json';expected='a8397c25842ef49a5288a50869b4ca614209062496e5d5d2bd08b70e13f64520';asset='croisement02-south-cart-initial-physical'
def guard():
 assert shutil.disk_usage(OUT).free>23*1024**3,'Free-space guard'
 if D.exists():assert sum(p.stat().st_size for p in D.rglob('*')if p.is_file())<=25*1024**2,'New-output cap'
acquire()
try:
 guard();assert sha(S)==expected;assert sha(A)=='534d552590823cbb5221e1ff52a082657360eb16f80ff5c42acfdb4980a2e3e8'
 decision=json.loads(A.read_text());assert any(m['asset_id']==asset and m['model_sha256']==expected for c in decision['decisions'] for m in c['members'])
 assert not D.exists();D.mkdir(parents=True);bpy.ops.wm.open_mainfile(filepath=str(S));scene=bpy.context.scene;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];assert len(objects)==29;bpy.ops.object.select_all(action='DESELECT');parts=[]
 for o in objects:
  matrix=o.matrix_world.copy();o.parent=None;o.matrix_world=matrix;o.hide_render=False;o.hide_set(False);o.select_set(True);o.data.calc_loop_triangles();v=[matrix@p.co for p in o.data.vertices];parts.append(dict(name=o.name,vertices=len(v),triangles=len(o.data.loop_triangles),bounds=[[min(p[i]for p in v)for i in range(3)],[max(p[i]for p in v)for i in range(3)]],materials=[m.name if m else None for m in o.data.materials]))
 file=D/'world.glb';bpy.ops.export_scene.gltf(filepath=str(file),export_format='GLB',use_selection=True,export_animations=False,export_yup=True,export_materials='EXPORT',export_extras=True)
 anchors=OUT/'restart2-state/remaining-local-origins-v1/manifest.json';anchor=json.loads(anchors.read_text())['anchors']['south-cart'];b=file.read_bytes();n,kind=struct.unpack_from('<II',b,12);g=json.loads(b[20:20+n]);sc=g['scenes'][g.get('scene',0)];node=len(g['nodes']);g['nodes'].append(dict(name='Reusable south-cart family origin',translation=[-v for v in anchor],children=sc['nodes']));sc['nodes']=[node];j=json.dumps(g,separators=(',',':')).encode();j+=b' '*((-len(j))%4);tail=b[20+n:];data=struct.pack('<III',0x46546c67,2,20+len(j)+len(tail))+struct.pack('<II',len(j),kind)+j+tail;local=D/'initial-wagon.glb';local.write_bytes(data);assert data[20+len(j):]==tail;guard()
 for o in list(scene.objects):
  if o.type=='MESH':bpy.data.objects.remove(o,do_unlink=True)
 bpy.ops.import_scene.gltf(filepath=str(local));imported=[o for o in scene.objects if o.type=='MESH'];roots=[o for o in scene.objects if o.name.startswith('Reusable south-cart family origin')];assert len(roots)==1;roots[0].location+=Vector((anchor[0],-anchor[2],anchor[1]));bpy.context.view_layer.update();assert len(imported)==29
 # Restore display ownership only; the derivative carries no new gameplay state.
 for o in imported:o['asset_group']=asset
 scene.render.engine='CYCLES';scene.cycles.samples=8;render(E/'views.json',D/'actual',width=384);buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(D/'actual'/f'view-{i}-textured.png'),check_existing=False);a=array('f',[0])*len(im.pixels);im.pixels.foreach_get(a);buffers.append(a);bpy.data.images.remove(im)
 _tile(buffers,384,384,D/'actual/textured.png');guard();assert sha(S)==expected
 write_json(D/'manifest.json',dict(status='Private derivative; actual material root review pending',asset_id=asset,source=str(S),source_sha256=expected,approval_receipt=str(A),approval_sha256=sha(A),world_glb_sha256=sha(file),glb=str(local),glb_sha256=sha(local),anchor=anchor,anchor_manifest_sha256=sha(anchors),binary_payload_unchanged_by_rebase=True,parts=parts,scope='Only approved initial wagon at unchanged world placement. No horses, animation, mission logic, collision, or state binding inferred.',new_output_bytes=sum(p.stat().st_size for p in D.rglob('*')if p.is_file()),free_bytes=shutil.disk_usage(OUT).free))
finally:release()
