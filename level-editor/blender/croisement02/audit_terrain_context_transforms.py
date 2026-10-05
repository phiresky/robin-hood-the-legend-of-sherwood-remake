"""Verify terrain context imports against exact source-world vertices."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Matrix
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS


def bank_world():
    return {o.get('source_node'):np.array([o.matrix_world@v.co for v in o.data.vertices]) for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'}


def main():
    output=OUT/'restart2-bank321/context-transform-audit-v1'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    original=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank/model.blend'
    assembly=OUT/'terrain-bank-candidate/integration/scene.blend'
    ground=OUT/'restart2-ground38/cumulative848-v1'
    candidate=OUT/'restart2-bank321/foot-candidate-v2/worker.blend'
    before={str(p):sha(p) for p in [original,assembly,ground/'model.blend',candidate]}
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(original));bpy.context.view_layer.update();expected=bank_world()
        bpy.ops.wm.open_mainfile(filepath=str(assembly));bpy.context.view_layer.update();actual=bank_world()
        differences={n:float(abs(actual[n]-v).max()) for n,v in expected.items()}
        if max(differences.values())>.001:raise ValueError('Historical bank context transform mismatch: '+str(differences))
        bpy.ops.wm.open_mainfile(filepath=str(ground/'model.blend'));bpy.context.view_layer.update()
        obj=bpy.data.objects['Croisement02 Terrain'];expected_ground=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);ground_matrix=np.array(obj.matrix_world)
        bpy.ops.wm.open_mainfile(filepath=str(candidate));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        names=json.loads((ground/'views.json').read_text())['object_names']
        with bpy.data.libraries.load(str(ground/'model.blend'),link=False) as (available,loaded):loaded.objects=names
        prelink={o.name:[list(r) for r in o.matrix_world] for o in loaded.objects}
        matrix=Matrix(json.loads((ground/'geometry-before.json').read_text())['matrix'])
        for obj in loaded.objects:
            scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
        # The production contact render forces a dependency update itself.
        # Evaluate explicitly here before comparing physical world coordinates.
        bpy.context.view_layer.update();obj=loaded.objects[0]
        actual_ground=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);error=float(abs(actual_ground-expected_ground).max())
        projected=np.column_stack((actual_ground[:,0],-actual_ground[:,1]*SIN-actual_ground[:,2]*COS))
        if error>.001:raise ValueError('Current ground context transform mismatch')
        write_json(output/'report.json',dict(status='PASS',models=before,historical_bank_context_max_world_error_by_part=differences,
            current_ground_context_max_world_error=error,current_ground_context_matrix=[list(r) for r in obj.matrix_world],
            source_ground_matrix=ground_matrix.tolist(),unlinked_library_matrices=prelink,
            current_ground_projected_corners=projected.tolist(),current_ground_z_range=[float(actual_ground[:,2].min()),float(actual_ground[:,2].max())],
            findings=['Historical bank context matches the source-world bank vertices after its link/update path.','Current root-reviewed foot contact loader assigns the frozen nonidentity ground matrix explicitly; it does not read the unlinked imported identity matrix.','Exact ground source-only restoration retains parent hierarchy and updates dependencies before signing geometry.','No reviewed context requires rerender for the unlinked-matrix failure.'],geometry_changed=False))
        if any(sha(Path(p))!=h for p,h in before.items()):raise ValueError('Frozen model changed')
        print(json.dumps({'bank_max_error':max(differences.values()),'ground_max_error':error}))
    finally:release()


if __name__=='__main__':main()
