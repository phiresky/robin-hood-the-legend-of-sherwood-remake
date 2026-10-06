"""Private approved bark export with exact multi-UV Boolean surface regions."""
from pathlib import Path
import sys,json,hashlib,shutil
import bpy
R=Path.cwd();sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from restart8_binary_material_partition import convert
import export_editor
O=R/'level-editor/work/croisement02-refinement';B=O/'restart8-five-bark-approved-export-v1';read=lambda p:json.loads(p.read_text());h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main(n):
 assert shutil.disk_usage(O).free>25*2**30;row=next(r for r in read(B/'approvals.json')['records']if r['asset_id']==f'croisement02-tree-{n}');model=Path(row['model']);assert h(model)==row['model_sha256'];approval=O/'restart3-review-batches/batch-v16/user-approval.json';assert h(approval)=='e350fc5e83775a6aec133c9346371b4699955081267d625dfe30a6813943e941';asset=row['asset_id'];D=B/f'tree-{n}-v1';D.mkdir(exist_ok=False);placement=O/'restart2-textures/batch10-private-level3d-exact-v2/placement-evidence';doc=read(placement/'expanded-document.json');matrices=read(placement/'part-matrices.json');entries=[o for o in doc['objects']if o.get('group')==asset];parts=sorted(o['id']for o in entries);pivots=[matrices[o['id']]['matrix'][12:15]for o in entries];assert pivots and all(p==pivots[0]for p in pivots);pivot=pivots[0];plan=read(O/'restart2-textures/batch10-static-export-plan-v1/plan.json');catalog=read(Path(plan['catalog']));level=read(O/'baseline/Croisement02.rhp.json');bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];working=bpy.data.collections.get('Croisement02 Working')or bpy.data.collections.new('Croisement02 Working');original=[];records=[]
 for ob in objects:
  if ob.name not in working.objects:working.objects.link(ob)
  images={node.image for m in ob.data.materials if m and m.use_nodes for node in m.node_tree.nodes if node.type=='TEX_IMAGE'and node.image};original.append(dict(object=ob.name,source_node=ob.get('source_node'),matrix=[list(x)for x in ob.matrix_world],vertices=len(ob.data.vertices),polygons=len(ob.data.polygons),images=[dict(name=im.name,packed_sha256=hashlib.sha256(bytes(im.packed_file.data)).hexdigest()if im.packed_file else None)for im in images]))
  if'crown'not in ob.name.lower():records.append(convert(ob,D/(hashlib.sha256(ob.name.encode()).hexdigest()[:12]+'-partition.npz')))
 folder=D/'exact';folder.mkdir();report=export_editor.export_editor('Croisement02',folder/'model.glb',asset_id=asset,standalone_pivot=pivot,include_hidden_objects=[o.name for o in objects if o.hide_render],catalog=catalog,level=level);assert sorted(p['node']for p in report['asset']['parts'])==parts;assert h(model)==row['model_sha256'];bytes_total=sum(p.stat().st_size for p in D.rglob('*')if p.is_file());assert bytes_total<200*2**20;result=dict(status='Private exact Boolean-partition export; analytic delivery and visual review pending',source_model=str(model),source_sha256=h(model),approval_sha256=h(approval),asset_id=asset,model_sha256=h(folder/'model.glb'),parts=parts,pivot=pivot,source_receivers=original,conversion_records=records,export=report,source_model_unchanged=True,exact_original_rgb_images=True,publication=False,bytes=bytes_total,authority_files={str(p):h(p)for p in [placement/'static-placement-pins.json',placement/'part-matrices.json',Path(plan['catalog']),model.parent/'preservation.json',model.parent/'root-review.json',model.parent/'ready-candidate-v1.json']});(D/'export.json').write_text(json.dumps(result,indent=2)+'\n');print(model,bytes_total,flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
