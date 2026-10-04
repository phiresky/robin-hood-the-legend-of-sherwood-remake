"""Reopen and verify private root scope against the immutable approved oak."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
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


def main(directory):
    directory=directory.resolve();evidence=json.loads((directory/'evidence.json').read_text());original=Path(evidence['original_model']);parent=Path(evidence['geometry_parent']) if evidence.get('geometry_parent') else None
    if sha(original)!=evidence['original_model_sha256'] or (parent and sha(parent)!=evidence['geometry_parent_sha256']) or sha(directory/'model.blend')!=evidence['model_sha256']:raise ValueError('Bound model changed')
    bpy.ops.wm.open_mainfile(filepath=str(original));expected=outside()
    parent_geometry=None
    if parent:
        bpy.ops.wm.open_mainfile(filepath=str(parent));parent_geometry=geometry(wood())
    bpy.ops.wm.open_mainfile(filepath=str(directory/'model.blend'))
    if outside()!=expected or (parent_geometry is not None and geometry(wood())!=parent_geometry):raise ValueError('Saved source-only rebake changed crown, other owner, or wood geometry')
    if not evidence.get('solid_only_preview',False):
        projection=json.loads((directory/'source-ownership.json').read_text())
        if projection['geometry_changed']:raise ValueError('Projection changed geometry')
        for path,digest in projection['source_mask_evidence'].items():
            if sha(Path(path))!=digest:raise ValueError('Source mask binding changed')
        for row in projection['objects']:
            provenance=row.get('texel_provenance')
            if provenance:
                path=Path(provenance['path'])
                if sha(path)!=provenance['sha256']:raise ValueError('Texel provenance changed')
                with np.load(path) as data:
                    if not set(np.unique(data['ownership'])).issubset({0,1}):raise ValueError('Unexpected generated wood texels')
    reports=[]
    for obj in wood():
        bm=bmesh.new();bm.from_mesh(obj.data);record=dict(object=obj.name,vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces))
        unseen=set(bm.verts);components=[]
        while unseen:
            seed=unseen.pop();stack=[seed];count=0
            while stack:
                vertex=stack.pop();count+=1
                for edge in vertex.link_edges:
                    other=edge.other_vert(vertex)
                    if other in unseen:unseen.remove(other);stack.append(other)
            components.append(count)
        record['connected_component_vertices']=sorted(components,reverse=True);bm.free()
        if record['nonmanifold_edges'] or record['degenerate_faces']:raise ValueError('Root topology failed')
        reports.append(record)
    result=dict(status='PASS',solid_only_preview=evidence.get('solid_only_preview',False),model_sha256=evidence['model_sha256'],original_model_sha256=evidence['original_model_sha256'],unchanged_crown_and_other_meshes=len(expected),same_wood_geometry_as_parent=True if parent_geometry is not None else None,wood_topology=reports,source_domain_sha256=evidence['source_domain_sha256'])
    write_json(directory/'reopened-verification.json',result);print(json.dumps(result,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--candidate',type=Path,default=OUT/'tree35-root-research/candidate-v5');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);main(args.candidate)
