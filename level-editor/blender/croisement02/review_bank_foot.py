"""Reopen bank-foot candidate, verify frozen appearance, and render ground contacts."""
import json,math
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from correct_bank_foot import digest,surface,hits
from workspace_components import appearance_state
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY
from review_bank_candidate import camera


def main():
    folder=OUT/'restart2-bank321/foot-candidate-v2';output=folder/'contact-review'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    validation=json.loads((folder/'validation.json').read_text())
    source=OUT/'texture-fill-round-2/croisement02-north-woodland-bank/complete-preparation/experiment-ground-retry-v2/bake-v1/worker.blend'
    ground=OUT/'restart2-ground38/cumulative848-v1'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source))
        bank=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
        names=[o.name for o in bank]
        appearance={o.name:digest(appearance_state(o,{})) for o in bank}
        coords={o.name:np.array([v.co[:] for v in o.data.vertices]) for o in bank}
        bpy.ops.wm.open_mainfile(filepath=str(folder/'worker.blend'))
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        bank=[bpy.data.objects[n] for n in names]
        expected={(r['object'],r['vertex']) for r in validation['moved_vertices']};actual=set()
        for obj in bank:
            if digest(appearance_state(obj,{}))!=appearance[obj.name]:raise ValueError('Reopened UV/material/image mismatch')
            for i,v in enumerate(obj.data.vertices):
                if np.max(abs(np.array(v.co[:])-coords[obj.name][i]))>1e-7:actual.add((obj.name,i))
        if actual!=expected:raise ValueError('Reopened changed-vertex domain differs')
        contacts=[]
        for name,i in sorted(actual):
            p=bpy.data.objects[name].matrix_world@bpy.data.objects[name].data.vertices[i].co
            if abs(p.z)>.001:raise ValueError('Bank toe lost contact with ground')
            contacts.append(dict(object=name,vertex=i,world=list(p),ground_z=0,gap=p.z))
        write_json(output/'reopened-preservation.json',dict(status='PASS',model_sha256=sha(folder/'worker.blend'),exact_uv_material_images=True,changed_vertex_domain_exact=True,changed_vertices=len(actual),ground_model_sha256=sha(ground/'model.blend'),ground_contact_points=contacts,maximum_ground_gap=max(abs(r['gap']) for r in contacts),prior_texture_approval_inherited=False))
        # Render against the exact cumulative receiver in a temporary scene.
        ground_names=json.loads((ground/'views.json').read_text())['object_names']
        with bpy.data.libraries.load(str(ground/'model.blend'),link=False) as (available,loaded):loaded.objects=ground_names
        matrix=Matrix(json.loads((ground/'geometry-before.json').read_text())['matrix'])
        for obj in loaded.objects:
            scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
        visible=set(bank+loaded.objects)
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=obj not in visible
        target=Vector((920,-440/SIN,0))
        neutral=bpy.data.materials.new('Temporary bank contact solid');neutral.diffuse_color=(.5,.5,.5,1)
        for i,angle in enumerate([0,math.pi/4,-math.pi/4]):
            direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN))
            camera(scene,target,direction,800,520,800)
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=neutral if mode=='solid' else None
                scene.render.filepath=str(output/f'contact-{i}-{mode}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        scene.view_layers[0].material_override=None
        if sha(folder/'worker.blend')!=validation['model_sha256']:raise ValueError('Candidate changed during contact review')
    finally:release()


if __name__=='__main__':main()
