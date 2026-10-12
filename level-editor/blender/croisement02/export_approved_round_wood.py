"""Privately export approved tree wood; crowns are replaced from installed GLBs later."""
from pathlib import Path
import sys,json,hashlib,shutil
import bpy
R=Path.cwd();B=R/'level-editor/work/active/croisement02';D=B/'approved-round-wood-integration-oct11-v1';A=B/'restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json'
sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from restart8_binary_material_partition import convert
import export_editor
read=lambda p:json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main(n):
 acquire(slots=2)
 try:
  assert n in [32,38];assert shutil.disk_usage(R).free>=10*1024**3
  assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>=6*1024**3
  assert sha(A)=='d435a76b08eb3ac616f1ae2ffcdf19a7875969e515dd01f65de1871ca886ef1d';asset=f'croisement02-tree-{n}';row=next(r for r in read(A)['decisions']if r['asset_id']==asset);model=Path(row['model']);assert sha(model)==row['model_sha256']
  target=D/f'tree-{n}-v1';target.mkdir(parents=True,exist_ok=False);live=R/'level-editor/library/3d-assets/croisement02'/asset;descriptor=read(live/'asset.json');pins={str(p):sha(p)for p in [A,model,live/'model.glb',live/'asset.json']};pivot=descriptor['source_origin_scene'];parts=sorted(p['node']for p in descriptor['parts'])
  plan=read(B/'restart2-textures/batch10-static-export-plan-v1/plan.json');catalog=read(Path(plan['catalog']));level=read(B/'baseline/Croisement02.rhp.json');bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];records=[]
  for ob in objects:
   if ob.get('projection_component')!='crown'and'crown'not in ob.name.lower():records.append(convert(ob,target/(hashlib.sha256(ob.name.encode()).hexdigest()[:12]+'-partition.npz')))
  folder=target/'exact';folder.mkdir();report=export_editor.export_editor('Croisement02',folder/'model.glb',asset_id=asset,standalone_pivot=pivot,include_hidden_objects=[o.name for o in objects if o.hide_render],catalog=catalog,level=level);assert sorted(p['node']for p in report['asset']['parts'])==parts
  assert all(sha(Path(p))==h for p,h in pins.items());(target/'export.json').write_text(json.dumps(dict(status='PRIVATE_WOOD_EXPORT_PENDING_INSTALLED_CROWN_RESTORATION',source_model=str(model),source_sha256=sha(model),asset_id=asset,parts=parts,pivot=pivot,model_sha256=sha(folder/'model.glb'),conversion_records=records,export=report,pins=pins,current_model=str(live/'model.glb'),current_descriptor=str(live/'asset.json'),scope='Use exported wood only. Historical crown discarded before delivery.'),indent=2)+'\n');print(asset,'exported',flush=True)
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
