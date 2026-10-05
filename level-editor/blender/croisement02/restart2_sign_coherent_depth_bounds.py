"""Read-only source-ray depth feasibility for complete foliage components."""
import sys
from pathlib import Path
import json
from collections import defaultdict
import bpy
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from restart2_sign_fragment_bounds import Surface


def main():
    source=OUT/'restart2-fence/sign-fragment-bounds-v1'
    dest=OUT/'restart2-fence/sign-coherent-depth-bounds-v1'
    dest.mkdir(exist_ok=False)
    results=[]
    for target in [5,7,8]:
        detail=json.loads((source/f'target-{target}.json').read_text())
        for asset,item in detail['inputs'].items():
            path=Path(item['worker'])/'model.blend'
            assert sha(path)==item['model_sha256']
            bpy.ops.wm.open_mainfile(filepath=str(path))
            objects=[bpy.data.objects[n] for n in item['objects']]
            foliage=[o for o in objects if any(m and m.get('foliage_physical_opacity') for m in o.data.materials)]
            assert foliage
            surface=Surface([(asset,o) for o in foliage])
            fragments=[f for f in detail['fragments'] if f['asset']==asset]
            shifts=defaultdict(float)
            for f in fragments:
                key=(f['object'],f['component'])
                shifts[key]=max(shifts[key],f['maximum_retreat'])
            allpoints=np.array([tuple(o.matrix_world@v.co) for o in foliage for v in o.data.vertices])
            lower,upper=allpoints.min(0),allpoints.max(0)
            maximum=max(shifts.values())
            refinement=Path(item['worker'])/'inspection/refinement.json'
            crown=json.loads(refinement.read_text())['crown']
            components=[]
            for key,shift in shifts.items():
                c=surface.components[key]
                components.append(dict(**c,required_retreat=shift,translation=(-np.array(RAY)*shift).tolist(),minimum_z_after=c['bounds_min'][2]-SIN*shift))
            branch=crown.get('branch_clumps')
            branch_bounds=[]
            if branch:
                centers=np.asarray(branch['centers']); radii=np.asarray(branch['radii'])
                demands=defaultdict(float)
                for f in fragments:
                    o=bpy.data.objects[f['object']]; polygon=o.data.polygons[f['polygon']]
                    midpoint=np.mean([tuple(o.matrix_world@o.data.vertices[i].co) for i in polygon.vertices],axis=0)
                    index=int(np.argmin(np.sum(((midpoint-centers)/radii)**2,axis=1)))
                    demands[index]=max(demands[index],f['maximum_retreat'])
                for i,shift in sorted(demands.items()):
                    branch_bounds.append(dict(branch=i,assignment='Nearest normalized ellipsoid; conceptual support only, not authoritative vertex ownership',required_retreat=shift,center=centers[i].tolist(),radii=radii[i].tolist(),shifted_center=(centers[i]-np.array(RAY)*shift).tolist(),ellipsoid_minimum_z_after=float(centers[i,2]-radii[i,2]-SIN*shift)))
            row=dict(target=target,asset=asset,input=item,detail_sha256=sha(source/f'target-{target}.json'),refinement_sha256=sha(refinement),whole_foliage=dict(bounds_min=lower.tolist(),bounds_max=upper.tolist(),width=float(upper[0]-lower[0]),depth=float(upper[1]-lower[1]),required_retreat=maximum,translation=(-np.array(RAY)*maximum).tolist(),raw_minimum_z_after=float(lower[2]-SIN*maximum),translation_preserves_dimensions=True),components=components,branch_bounds=branch_bounds)
            if crown.get('opacity_bounds'):
                b=crown['opacity_bounds'];row['whole_foliage']['opaque_minimum_z_after']=b['bounds_min'][2]-SIN*maximum
                row['whole_foliage']['ground_clearance_retreat_limit']=b['bounds_min'][2]/SIN
            row['local_bend_constraints']=dict(affected_components=len(components),components_raw_vertices_below_world_zero=sum(c['minimum_z_after']<0 for c in components),maximum_retreat=maximum,maximum_vertical_drop=SIN*maximum,maximum_horizontal_shift=COS*maximum,retreat_over_current_depth=maximum/(upper[1]-lower[1]),ground_anchor_must_remain_fixed=True,paired_backs_and_interiors_must_follow_same_continuous_depth_field=True)
            write_json(dest/f'target-{target}-{asset}.json',row)
            results.append({k:v for k,v in row.items() if k!='components'})
            assert sha(path)==item['model_sha256']
    write_json(dest/'report.json',dict(status='Read-only coherent depth bounds; no geometry changes',source_report_sha256=sha(source/'report.json'),results=results,interpretation=['Observed native texture ownership does not establish physical depth. Source-ray translation preserves vertex projection algebraically.','Whole-volume translations lower grounded shrubs below their existing support; they are unsuitable without envelope reconstruction.','Disconnected microcards are not independent biological branches. Component bounds are lower-level diagnostics, not a proposed scattered-card fix.','A continuous local bend may still be possible. It must preserve ground/wood support, paired backs, native alpha/RGB and all poses; these numeric bounds do not prove universal impossibility.','Bounds use complete card vertices and can include transparent margins. Negative raw bounds alone do not prove opaque penetration.','Only four sign poses and canopy frame0 were sampled; final reconstruction needs all32 sign poses and native canopy phase coverage.']))
    print(dest)

if __name__=='__main__':
    acquire()
    try: main()
    finally: release()
