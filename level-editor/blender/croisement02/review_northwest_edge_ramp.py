"""Reopen a bounded rock-edge trial, verify its scope, and render frozen views."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from workspace_components import appearance_state
from refinement_workspace import _geometry
from correct_bank_foot import digest
from tree_geometry import SIN,COS,RAY
from render_multiview_asset import render


def main():
    folder=OUT/'restart2-bank321/northwest-edge-ramp-v1';output=folder/'reopened-review'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    source=OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/worker.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
        objects=[o for o in bpy.data.objects if o.type=='MESH']
        geometry={o.name:digest(_geometry(o)) for o in objects}
        appearance={o.name:digest(appearance_state(o,{})) for o in objects if o.get('asset_group')=='croisement02-northwest-rock-outcrop'}
        obj=next(o for o in objects if o.get('source_node')=='building-035')
        name=obj.name;before=np.array([obj.matrix_world@v.co for v in obj.data.vertices])
        bpy.ops.wm.open_mainfile(filepath=str(folder/'worker.blend'));bpy.context.view_layer.update()
        for name_,state in appearance.items():
            if digest(appearance_state(bpy.data.objects[name_],{}))!=state:raise ValueError('Appearance changed on reopen')
        for name_,state in geometry.items():
            if name_!=name and digest(_geometry(bpy.data.objects[name_]))!=state:raise ValueError('Unrelated geometry changed')
        obj=bpy.data.objects[name];after=np.array([obj.matrix_world@v.co for v in obj.data.vertices])
        expected=before+np.clip((before[:,0]-80)/25,0,1)[:,None]*51.5*np.array(RAY)[None,:]
        error=float(np.max(abs(after-expected)))
        if error>.0001:raise ValueError('Reopened deformation differs from bounded ramp')
        projection=lambda p:np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS))
        moved=np.max(abs(after-before),axis=1)>.0001
        write_json(output/'preservation.json',dict(status='PASS',model_sha256=sha(folder/'worker.blend'),source_sha256=sha(source),changed_object=name,moved_vertices=int(moved.sum()),unchanged_vertices=int((~moved).sum()),maximum_world_ramp_error=error,maximum_source_projection_error=float(np.max(abs(projection(after)-projection(before)))),uv_material_image_bytes_exact=True,other_mesh_geometry_exact=True,source_ownership_changed=False,user_approval=None))
        packet=json.loads((source.parent.parent/'views.json').read_text())
        normal=np.array(packet['views'][0]['camera_matrix_world'])[:3,2]
        if np.max(abs(normal-np.array(RAY)))>.000001:raise ValueError('First view is not the native source camera')
        write_json(output/'views.json',packet)
        scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
        render(output/'views.json',output/'actual',width=384)
        sheet=Image.new('RGB',(1536,768))
        for i in range(8):sheet.paste(Image.open(output/'actual'/f'view-{i}-textured.png').convert('RGB'),((i%4)*384,(i//4)*384))
        sheet.save(output/'actual8.png')
    finally:release()


if __name__=='__main__':main()
