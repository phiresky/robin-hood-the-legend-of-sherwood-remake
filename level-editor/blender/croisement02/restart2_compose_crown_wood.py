"""Compose a frozen crown trial with exact scoped selected wood and bind both sources."""
import argparse,json,sys,shutil,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry,validate,_render
from workspace_components import appearance_state
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release

def crown_state(obj):
 appearance=appearance_state(obj)
 for material in appearance['materials']:
  if not material:continue
  material.pop('name',None)
  for node in material.get('nodes',[]):
   if 'image' in node:node['image'].pop('name',None)
 return dict(geometry=_geometry(obj,False),appearance_sha256=hashlib.sha256(json.dumps(appearance,sort_keys=True).encode()).hexdigest())

def main():
 p=argparse.ArgumentParser();p.add_argument('--index',type=int,choices=[31,32,35,38,43,45,46],required=True);p.add_argument('--crown',type=Path,required=True);p.add_argument('--crown-sha',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);crown=a.crown.resolve();wood=tree_workspace(a.index);worker=a.output.resolve();wood_hash=sha(wood/'model.blend')
 if worker.name!=wood.name or crown.name!=wood.name or sha(crown/'model.blend')!=a.crown_sha:raise ValueError('Frozen inputs mismatch')
 proof=crown/'inspection/envelope-preservation.json';record=json.loads(proof.read_text())
 if record['model_sha256']!=a.crown_sha or not record['original_native_rgba_image_reused_exactly']:raise ValueError('Crown source proof invalid')
 bpy.ops.wm.open_mainfile(filepath=str(crown/'model.blend'));bpy.context.view_layer.update();crowns=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==wood.name and o.get('projection_component')=='crown'];expected={o.name:crown_state(o) for o in crowns};matrices={o.name:o.matrix_world.copy() for o in crowns}
 if not crowns:raise ValueError('Missing incoming crown')
 bpy.ops.wm.open_mainfile(filepath=str(wood/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;allobjects=list(bpy.data.collections['Croisement02 Working'].all_objects);before={o.name:_geometry(o,True) for o in allobjects if o.name not in expected};wood_before={o.name:_geometry(o,True) for o in allobjects if o.type=='MESH' and o.get('asset_group')==wood.name and o.get('projection_component')!='crown'}
 names=list(expected)
 with bpy.data.libraries.load(str(crown/'model.blend'),link=False) as (_,loaded):loaded.objects=list(names)
 for name,source in zip(names,loaded.objects):
  if source is None:raise ValueError('Missing crown mesh')
  target=bpy.data.objects[name];target.data=source.data;target.matrix_world=matrices[name]
 for source in loaded.objects:bpy.data.objects.remove(source,do_unlink=True)
 bpy.context.view_layer.update();actual={name:crown_state(bpy.data.objects[name]) for name in names}
 if actual!=expected:raise ValueError('Incoming crown changed during composition')
 if before!={o.name:_geometry(o,True) for o in allobjects if o.name not in expected}:raise ValueError('Selected wood or outside scene changed')
 shutil.copytree(wood,worker);(worker/'modified').rename(worker/'previous-modified');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));validate(worker);cfg=json.loads((worker/'workspace.json').read_text());_render(cfg,worker/'modified',worker/'input/views.json');validate(worker)
 write_json(worker/'inspection/crown-wood-composition.json',dict(model_sha256=sha(worker/'model.blend'),wood_worker=str(wood),wood_model_sha256=wood_hash,crown_worker=str(crown),crown_model_sha256=a.crown_sha,crown_proof_sha256=sha(proof),crown_before=expected,crown_after=actual,wood_before=wood_before,wood_after={o.name:_geometry(o,True) for o in allobjects if o.type=='MESH' and o.get('asset_group')==wood.name and o.get('projection_component')!='crown'},non_crown_geometry_appearance_exact=True,source_mask_sha256=sha(worker/'source-masks.json'),status='Private combined candidate; native joint and root review pending',approval='pending'))
 audit(worker);render_workspace(worker,384,release_slot=False,transparent_bounces=256)
 if sha(wood/'model.blend')!=wood_hash or sha(crown/'model.blend')!=a.crown_sha:raise ValueError('Frozen input changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
