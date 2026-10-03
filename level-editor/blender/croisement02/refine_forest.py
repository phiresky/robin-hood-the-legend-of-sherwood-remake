"""Build isolated tree geometry candidates with frozen source-only review packets.

Touching crown boundaries are reviewable ownership hypotheses. Each worker
retains the source scene and checks every outside asset after its own changes.
"""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,TREES
from render_slots import acquire,release
from evidence_io import sha
from refinement_workspace import prepare,validate,modified
from tree_geometry import crown_geometry,wood_geometry,replace_mesh
from bark_materials import fill


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--masks',nargs='*',type=int)
    parser.add_argument('--redo',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    rows=json.loads((OUT/'forest-v4-sources/manifest.json').read_text());level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    traces={r['mask']:r for r in json.loads((OUT/'wood-traces.json').read_text())}
    review=dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'forest-v4-inventory/inventory.json'),
        evidence='Six obstacle sheets, both native depth layers and three forest-source sheets inspected. Wood identities are reviewed. Shared canopy partitions are explicitly inferred construction candidates, not recovered ownership or approved final trees.')
    review_path=OUT/'forest-v4-grouping-review.json'
    if not review_path.exists():review_path.write_text(json.dumps(review,indent=2)+'\n')
    inv=json.loads((OUT/'baseline/masks/manifest.json').read_text());assignments=[]
    for record in inv['masks']:
        if record['png']:record['png']=str(OUT/'baseline/masks'/record['png'])
    for offset,r in enumerate(rows):
        packet=json.loads(Path(r['packet']).read_text());image=Image.open(Path(r['packet']).parent/'complete-source.png')
        alpha=Path(r['packet']).parent/'coverage.png'
        if not alpha.exists():image.getchannel('A').save(alpha)
        frame=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'][r['animation']]['frames'][0]
        index=142+offset
        inv['masks'].append(dict(index=index,layer=0,png=str(alpha),box_top_left=packet['native_bbox'][:2],box_size=packet['native_bbox'][2:],provenance='Native canopy alpha intersected with inferred rounded tree supports'))
        assignments.append(dict(reviewed=True,asset_group=r['asset_id'],mask_indices=[r['mask']]))
        assignments.append(dict(reviewed=True,source_node=f"building-{r['primary']:03}",projection_component='crown',mask_indices=[index]))
    inventory_path=OUT/'forest-v4-mask-inventory.json'
    if not inventory_path.exists():inventory_path.write_text(json.dumps(inv,indent=2)+'\n')
    mask_path=OUT/'forest-v4-source-masks.json'
    if not mask_path.exists():mask_path.write_text(json.dumps(dict(version=1,mask_inventory=str(inventory_path),projections={'exterior':dict(source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),state='initial, synchronized first frame; authored crown partitions provisional',assignments=assignments)}),indent=2)+'\n')
    for r in rows:
        if args.masks is not None and r['mask'] not in args.masks:continue
        workspace=OUT/'forest-v4-round-1/assets'/r['asset_id']
        if (workspace/'inspection/refinement.json').exists() and not args.redo:
            previous=json.loads((workspace/'inspection/refinement.json').read_text())
            if previous['crown'].get('geometry_version')=='native-leaf-clusters-v5':continue
        acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'));bpy.context.preferences.filepaths.save_version=0
        if (workspace/'workspace.json').exists():
            bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'));validate(workspace)
        else:
            prepare(workspace,asset_id=r['asset_id'],scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',
                    source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=review_path,
                    source_mask_manifest=mask_path,width=192,height=256,framing_padding=1.25,
                    lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==r['asset_id']]
        wood=[o for o in objects if o.get('projection_component')!='crown'];paths=traces[r['mask']]['paths'];assign={o.name:[] for o in wood}
        # Assign each traced path to an existing canonical part by its source
        # silhouette centre. Group identity was reviewed independently above.
        centres=[]
        for obj in wood:
            i=int(obj['source_node'].split('-')[-1]);pts=level['sight_obstacles'][i]['points']
            centres.append((np.mean([p['x'] for p in pts]),np.mean([p['y']-(p['z_top']+p['z_bottom'])/2 for p in pts])))
        for path in paths:
            center=np.mean(np.asarray(path)[:,:2],axis=0)
            winner=int(np.argmin(np.sum((np.asarray(centres)-center)**2,axis=1)));assign[wood[winner].name].append(path)
        reports=[]
        for obj in wood:
            if not assign[obj.name]:
                reports.append(dict(source_node=obj['source_node'],status='native part retained; no distinct traced branch assigned'));continue
            vertices,faces=wood_geometry(assign[obj.name],r['ground_y'])
            report=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials));report['source_node']=obj['source_node']
            if report['nonmanifold_edges']:raise ValueError('Open wood tube')
            for face in obj.data.polygons:face.use_smooth=len(face.vertices)==4
            reports.append(report)
        crown=next(o for o in objects if o.get('projection_component')=='crown')
        crown_report=crown_geometry(crown,json.loads(Path(r['packet']).read_text()),r['ground_y'],True)
        if crown_report['depth']<crown_report['width']*.999:raise ValueError('Flattened crown')
        validate(workspace);modified(workspace)
        bark=fill(workspace,objects,r['mask'])
        validate(workspace);bpy.ops.wm.save_as_mainfile(filepath=str(workspace/'model.blend'))
        (workspace/'inspection').mkdir(exist_ok=True)
        report=dict(asset_id=r['asset_id'],mask=r['mask'],wood=reports,crown=crown_report,model_sha256=sha(workspace/'model.blend'),
            bark=bark,status='geometry candidate; visual review pending',limitations=['Rounded overlapping supports divide native canopy clusters; individual crown ownership and hidden leaf depth are inferred.','Wood radius comes from mask distance; hidden branch junctions and round cross-sections are inferred.','No AI fill or publication performed.','Only synchronized first-frame foliage is modeled; all authored animation frames remain preserved as evidence.'])
        (workspace/'inspection/refinement.json').write_text(json.dumps(report,indent=2)+'\n')
        (workspace/'review.md').write_text('Native wood mask '+str(r['mask'])+'; separate curved foliage with depth at least width. Source-only gray means unobserved texture. Shared canopy boundaries and hidden completion require visual review. No publication or geometry approval is implied.\n')
        print('COMPLETED',r['asset_id'],flush=True);release()

if __name__=='__main__':main()
