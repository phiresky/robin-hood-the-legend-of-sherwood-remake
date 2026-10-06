"""Export the exact approved static tree candidate to a private asset library."""
import json,sys,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from export_editor import export_asset_library
from refinement_workspace import _geometry
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';ASSET='croisement03-tree-25'
def main():
 approval=B/'texture-approval-v12-tree25-cluster-v6/scope.json';decision=json.loads(approval.read_text());source=Path(decision['model']);assert decision['status']=='approved' and decision['scope']=='texture';assert sha(source)==decision['model_sha256']=='9e8ece3cc693a2ef387333c762e8a85d2cc3fa808a7726c821bebd9233cb0272';out=B/'tree25-cluster-integration-v1';assert not out.exists();assert shutil.disk_usage(ROOT).free>25*1024**3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));objects=[o for o in bpy.data.collections['Croisement03 Working'].objects if o.type=='MESH' and o.get('asset_group')==ASSET and not o.hide_render];assert len(objects)==1;before={o.name:_geometry(o,protect_appearance=True) for o in objects};out.mkdir();result=export_asset_library('Croisement03',out/'assets',B.parent/'baseline/Croisement03.rhp.json',asset_ids=[ASSET]);assert before=={o.name:_geometry(o,protect_appearance=True) for o in objects};assert sha(source)==decision['model_sha256'];write_json(out/'export-proof.json',dict(status='Private exact approved static export; derivative appearance/gameplay/browser checks pending',model_sha256=sha(source),approval_sha256=sha(approval),export=result,geometry_and_appearance_unchanged=True,source_objects=[dict(name=o.name,source_node=o.get('source_node'),asset_group=o.get('asset_group')) for o in objects],files={str(p.relative_to(out)):sha(p) for p in out.rglob('*') if p.is_file()},limits=['Static geometry/appearance approved; wind/runtime dynamic integration remains separate.','No live map or index write.']));print(out)
 finally:release()
if __name__=='__main__':main()
