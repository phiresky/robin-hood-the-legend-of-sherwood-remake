"""Measure crown geometry on the current tree00 and the two permitted references."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import RAY,SIN,COS


def measure(obj):
    matrix=obj.matrix_world
    xyz=np.array([matrix@v.co for v in obj.data.vertices])
    normal=matrix.to_3x3().inverted().transposed()
    axes=np.array([(1,0,0),(0,-SIN,-COS),tuple(RAY)])
    rows=[]
    for face in obj.data.polygons:
        n=np.array((normal@face.normal).normalized())
        positions=xyz[list(face.vertices)]
        center=np.mean(positions,axis=0)
        lengths=[float(np.linalg.norm(positions[(i+1)%len(positions)]-v)) for i,v in enumerate(positions)]
        rows.append(dict(axis_aligned=float(np.max(np.abs(axes@n)))>.99999,
                         in_map=float(-center[1]*SIN-center[2]*COS)>=0,
                         max_edge=max(lengths),minimum_edge=min(lengths),
                         observed=bool(obj.data.materials[face.material_index].get('foliage_observed'))))
    inmap=[r for r in rows if r['in_map']]
    return dict(vertices=len(xyz),faces=len(rows),world_span=np.ptp(xyz,axis=0).tolist(),
        source_basis_aligned_faces=sum(r['axis_aligned'] for r in rows),
        inmap_faces=len(inmap),inmap_source_basis_aligned_faces=sum(r['axis_aligned'] for r in inmap),
        edge_p50_p95_max=np.percentile([r['max_edge'] for r in rows],[50,95,100]).tolist(),
        material_slots=[dict(name=m.name,physical_opacity=bool(m.get('foliage_physical_opacity')),
            observed=bool(m.get('foliage_observed')),paired_one_sided=m.get('foliage_card_sides')) for m in obj.data.materials])


def main():
    acquire()
    try:
        old=tree_workspace(0)/'model.blend'
        bpy.ops.wm.open_mainfile(filepath=str(old))
        crown=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-00' and o.get('projection_component')=='crown')
        report=dict(tree00=dict(model=str(old),model_sha256=sha(old),crown=measure(crown)),references=[])
        for asset in ('leicester-southeast-cottage-tree','leicester-moat-bank-tree'):
            path=ROOT/'level-editor/library/3d-assets/leicester'/asset/'model.glb'
            scene=bpy.data.scenes.new(asset);bpy.context.window.scene=scene
            bpy.ops.import_scene.gltf(filepath=str(path))
            crowns=[o for o in scene.objects if o.type=='MESH' and 'crown' in o.name.lower()]
            report['references'].append(dict(asset_id=asset,model=str(path),sha256=sha(path),crowns=[measure(o) for o in crowns]))
        report['diagnosis']=[
            'Existing in-map foliage is repeated in a screen-aligned grid across three depth samples. All three crossed planes share the source basis; regular orientation and depth grouping remain visible at oblique angles.',
            'The generator scales every vertex along the source ray to reach a width/depth target, affecting the thickness of leaf clusters as well as their placement. Ground constraints also collect lower patches near a common plane.',
            'These features are geometric. Material fill cannot remove aligned plane silhouettes or repeated placement. Front-native projection agreement alone does not validate volumetric tree quality.',
            'The two permitted references use curved foliage surfaces with changing normals. Their shallower depth must not be copied; the prototype keeps a full-depth envelope and distributes small clusters irregularly.',
            'The private integration exact-preservation check rules out texture import changing the selected approved models. No sun/light calibration change is proposed to conceal geometry.']
        write_json(OUT/'canopy-representation-diagnosis.json',report)
        print(json.dumps({k:v for k,v in report['tree00']['crown'].items() if k!='material_slots'}))
    finally:release()


if __name__=='__main__':main()
