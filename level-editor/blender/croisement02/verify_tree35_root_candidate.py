"""Reopen and verify private root scope against the immutable approved oak."""
import json,sys
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from project_tree35_root_candidate import geometry


def outside():
    return {o.name:_geometry(o,protect_appearance=True) for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not (o.get('asset_group')=='croisement02-tree-35' and o.get('projection_component')!='crown')}


def wood():
    return [o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-35' and o.get('projection_component')!='crown']


def main():
    directory=OUT/'tree35-root-research/candidate-v5';evidence=json.loads((directory/'evidence.json').read_text());original=Path(evidence['original_model']);parent=Path(evidence['geometry_parent'])
    if sha(original)!=evidence['original_model_sha256'] or sha(parent)!=evidence['geometry_parent_sha256'] or sha(directory/'model.blend')!=evidence['model_sha256']:raise ValueError('Bound model changed')
    bpy.ops.wm.open_mainfile(filepath=str(original));expected=outside()
    bpy.ops.wm.open_mainfile(filepath=str(parent));parent_geometry=geometry(wood())
    bpy.ops.wm.open_mainfile(filepath=str(directory/'model.blend'))
    if outside()!=expected or geometry(wood())!=parent_geometry:raise ValueError('Saved source-only rebake changed crown, other owner, or wood geometry')
    reports=[]
    for obj in wood():
        bm=bmesh.new();bm.from_mesh(obj.data);record=dict(object=obj.name,vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free()
        if record['nonmanifold_edges'] or record['degenerate_faces']:raise ValueError('Root topology failed')
        reports.append(record)
    result=dict(status='PASS',model_sha256=evidence['model_sha256'],original_model_sha256=evidence['original_model_sha256'],unchanged_crown_and_other_meshes=len(expected),same_wood_geometry_as_parent=True,wood_topology=reports,source_domain_sha256=evidence['source_domain_sha256'])
    write_json(directory/'reopened-verification.json',result);print(json.dumps(result,indent=2))

if __name__=='__main__':main()
