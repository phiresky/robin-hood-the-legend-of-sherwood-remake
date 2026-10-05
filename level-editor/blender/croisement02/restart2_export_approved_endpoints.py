"""Export independently approved physical endpoints without inventing transition animation."""
import sys,json
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
DEST=OUT/'restart2-state/approved-physical-endpoint-exports-v1'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 source=OUT/'restart2-textures/batch-v2-texture-approval-v1/decisions.json';decisions=json.loads(source.read_text())['decisions'];wanted=[r for r in decisions if any(s in r['asset_id'] for s in ['rock-trap','log-trap','net-piege01'])]
 if len(wanted)!=5:raise ValueError('Expected five independently approved endpoints')
 acquire()
 try:
  DEST.mkdir();records=[]
  for row in wanted:
   path=Path(row['candidate'])/'worker.blend'
   if row['decision']!='approved' or sha(path)!=row['model_sha256']:raise ValueError('Approval binding changed')
   bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==row['asset_id']]
   if not objects:raise ValueError('Missing approved endpoint meshes '+row['asset_id'])
   bpy.ops.object.select_all(action='DESELECT');parts=[]
   for obj in objects:
    matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False;obj.hide_set(False);obj.select_set(True);obj.data.calc_loop_triangles();points=[matrix@v.co for v in obj.data.vertices];parts.append({'name':obj.name,'vertices':len(points),'triangles':len(obj.data.loop_triangles),'bounds':[[min(p[i]for p in points)for i in range(3)],[max(p[i]for p in points)for i in range(3)]],'materials':[m.name if m else None for m in obj.data.materials]})
   file=DEST/(row['asset_id']+'.glb');bpy.ops.export_scene.gltf(filepath=str(file),export_format='GLB',use_selection=True,export_animations=False,export_yup=True,export_materials='EXPORT',export_extras=True)
   if sha(path)!=row['model_sha256']:raise ValueError('Source modified')
   records.append({'id':row['asset_id'],'model_source':str(path),'model_sha256':row['model_sha256'],'glb':file.name,'glb_sha256':sha(file),'parts':parts,'triangles':sum(p['triangles']for p in parts),'scope':'One physical endpoint at reviewed world placement; no transition clip or mission activation inferred.'})
  write_json(DEST/'manifest.json',{'status':'Private approved endpoint export; browser/material verification pending','approval_ledger_sha256':sha(source),'records':records,'limits':['Separate endpoint exports only.','No physical temporal correspondence or script execution.','No canonical publication.']})
 finally:release()
if __name__=='__main__':main()
