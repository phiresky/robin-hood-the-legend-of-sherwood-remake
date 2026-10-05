"""Validate saved candidates and bind the inspection recipe to their model hashes."""
import json
import sys
from pathlib import Path
from collections import Counter
import bpy
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import RAY
from evidence_io import sha,record_recipe
from refinement_workspace import validate
from render_slots import acquire,release


def audit(workspace, *, inferred_constant_materials=()):
    validate(workspace)
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==workspace.name]
    records=[]
    for obj in objects:
        matrix=obj.matrix_world.to_3x3().inverted().transposed();used={p.material_index for p in obj.data.polygons}
        if not all(np.isfinite(tuple(v.co)).all() for v in obj.data.vertices):raise ValueError('Nonfinite geometry')
        materials=[]
        for index in used:
            mat=obj.data.materials[index]
            images=[n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
            if not images and mat.name in inferred_constant_materials:
                principled=mat.node_tree.nodes.get('Principled BSDF')
                if principled is None or any(socket.is_linked for socket in principled.inputs):
                    raise ValueError('Declared constant material is not constant: '+mat.name)
                materials.append(dict(name=mat.name,images=[],source_role='Explicitly inferred constant hidden surface',
                                      base_color=list(principled.inputs['Base Color'].default_value)))
                continue
            if not images or not all(i.packed_file for i in images):raise ValueError('Unpacked or untextured used material: '+obj.name)
            materials.append(dict(name=mat.name,images=[dict(name=i.name,size=list(i.size),packed_sha256=__import__('hashlib').sha256(i.packed_file.data).hexdigest()) for i in images]))
        row=dict(object=obj.name,source_node=obj['source_node'],vertices=len(obj.data.vertices),faces=len(obj.data.polygons),used_materials=materials)
        if obj.get('projection_component')=='crown':
            observed=[p for p in obj.data.polygons if obj.data.materials[p.material_index].get('foliage_observed')]
            wrong=[p.index for p in observed if (matrix@p.normal).normalized().dot(RAY)<.05]
            if wrong:
                diagnostic=dict(materials=[dict(slot=i,name=m.name,observed=m.get('foliage_observed'),faces=sum(p.material_index==i for p in obj.data.polygons)) for i,m in enumerate(obj.data.materials)],wrong=[dict(face=p.index,slot=p.material_index,normal=list(p.normal),world_cosine=float((matrix@p.normal).normalized().dot(RAY))) for p in observed if p.index in set(wrong)][:8])
                (workspace/'inspection/normal-diagnostic.json').write_text(json.dumps(diagnostic,indent=2)+'\n')
                raise ValueError('Observed faces point away from source: '+obj.name)
            for p in observed:
                mat=obj.data.materials[p.material_index]
                if not mat.use_backface_culling or not mat.node_tree.nodes.get('One-sided foliage'):raise ValueError('Front/rear culling differs between renderers')
            ownership=obj.data.color_attributes['Source ownership']
            for p in obj.data.polygons:
                expected=1. if obj.data.materials[p.material_index].get('foliage_observed') else 0.
                if any(abs(ownership.data[i].color[0]-expected)>1e-5 for i in p.loop_indices):raise ValueError('Leaf ownership does not match face material')
            counts=Counter(tuple(sorted(edge)) for p in obj.data.polygons for edge in p.edge_keys)
            row.update(observed_faces=len(observed),wrong_source_normals=0,boundary_edges=sum(n==1 for n in counts.values()),nonmanifold_edges=sum(n!=2 for n in counts.values()),boundary_reason='Intentional foliage cutouts and separate paired backs')
        records.append(row)
    result=dict(status='PASS',model_sha256=sha(workspace/'model.blend'),objects=records,
                ownership_note='Opaque bark alpha is not ownership evidence. Use the baker\'s explicit per-texel provenance NPZ for source versus inferred bark.',
                inspection_recipe=record_recipe(workspace,__file__))
    (workspace/'inspection/saved-model-audit.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


def main():
    completed=0
    for root in ['forest-v4-round-1','scenery-round-1']:
        for workspace in sorted((OUT/root/'assets').iterdir()):
            path=workspace/'inspection/refinement.json'
            if not path.exists():continue
            record=json.loads(path.read_text())
            if root.startswith('forest') and record['crown'].get('geometry_version')!='native-leaf-clusters-v5':continue
            if record['model_sha256']!=sha(workspace/'model.blend'):raise ValueError('Worker is being modified: '+workspace.name)
            existing=workspace/'inspection/saved-model-audit.json'
            if existing.exists() and json.loads(existing.read_text())['model_sha256']==record['model_sha256']:continue
            acquire();bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'));audit(workspace)
            completed+=1;print('AUDITED',workspace.name,flush=True)
            if completed%8==0:release()
    release()

if __name__=='__main__':main()
