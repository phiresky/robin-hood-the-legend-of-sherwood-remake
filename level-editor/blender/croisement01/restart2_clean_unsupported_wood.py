"""Remove only disconnected wood crumbs proven to own zero native source rays."""
import argparse,hashlib,json,os,shutil,sys
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';p=argparse.ArgumentParser();p.add_argument('--tree',type=int,choices=[4,5],required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);v={4:6,5:7}[a.tree];asset=f'croisement01-tree-{a.tree:02d}';source=R/f'tree{a.tree:02d}-v{v}';dest=R/f'tree{a.tree:02d}-v{v+1}';w=source/'assets'/asset;out=dest/'assets'/asset;proof=json.loads((w/'inspection/independent-wood-coverage-v2.json').read_text());assert sha(w/'model.blend')==proof['model_sha256'];assert not dest.exists();assert all(h==0 for row in proof['topology'] for n,h in zip(row['components'],row['component_source_hits']) if n<100);acquire(slots=2)
try:
 dest.mkdir();out.mkdir(parents=True)
 for file in source.iterdir():
  if file.is_file():shutil.copy2(file,dest/file.name)
 for file in w.iterdir():
  if file.name in ['model.blend','inspection'] or file.name.endswith('.blend1'):continue
  target=out/file.name
  if file.is_dir():shutil.copytree(file,target,copy_function=os.link)
  else:shutil.copy2(file,target)
 cfg=json.loads((out/'workspace.json').read_text());cfg=json.loads(json.dumps(cfg).replace(str(w),str(out)));(out/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n')
 bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 images={im.name:hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file};removed=[]
 for o in bpy.context.scene.objects:
  if o.type!='MESH' or o.get('source_node')!=f'scenery-tree{a.tree:02d}-wood':continue
  bm=bmesh.new();bm.from_mesh(o.data);todo=set(bm.verts);components=[]
  while todo:
   stack=[todo.pop()];component=[]
   while stack:
    vertex=stack.pop();component.append(vertex)
    for edge in vertex.link_edges:
     other=edge.other_vert(vertex)
     if other in todo:todo.remove(other);stack.append(other)
   components.append(component)
  small=[c for c in components if len(c)<100];assert sorted(map(len,small))==sorted(n for row in proof['topology'] for n in row['components'] if n<100)
  for c in small:removed.append(dict(vertices=len(c),bounds=[[min(v.co[i] for v in c) for i in range(3)],[max(v.co[i] for v in c) for i in range(3)]]))
  bmesh.ops.delete(bm,geom=[v for c in small for v in c],context='VERTS');bm.to_mesh(o.data);bm.free();o.data.update()
 assert images=={im.name:hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file}
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));assert sha(w/'model.blend')==proof['model_sha256'];report=dict(status='CLEANED_REQUIRES_INDEPENDENT_COVERAGE_AND_VIEWS',source_model_sha256=proof['model_sha256'],model_sha256=sha(out/'model.blend'),removed=removed,packed_images_exact=images,source_support_proof_sha256=sha(w/'inspection/independent-wood-coverage-v2.json'))
 (dest/'cleanup.json').write_text(json.dumps(report,indent=2)+'\n');construction=json.loads((dest/'construction.json').read_text());construction['model_sha256']=report['model_sha256'];construction['cleanup']='Removed only disconnected remesh crumbs with zero native source ray hits';(dest/'construction.json').write_text(json.dumps(construction,indent=2)+'\n')
 (out/'inspection').mkdir(exist_ok=True)
 import render_candidate;sys.argv=['render','--',str(out)];render_candidate.main()
finally:release()
