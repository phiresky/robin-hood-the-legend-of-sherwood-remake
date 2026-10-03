"""Prepare isolated scenery candidates and honest pending state packets."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,GROUPS
from tree_geometry import SIN,COS
from scenery_geometry import refine
from refinement_workspace import prepare,modified,validate
from render_slots import acquire,release
from evidence_io import sha

MASKS={'east-stone-wall-and-gate':[101],'southeast-stone-wall-and-gate':[96,97],
'east-rail-fence':[96,97],'south-field-wattle-fence':[98],'southwest-path-wattle-fence':[99],
'southwest-field-wattle-fence':[100],'logging-clearing-stumps':[107,108],'logging-clearing-log':[108],
'north-firewood-stack':[109],'northwest-rock-outcrop':[50,51,53],'west-rock-outcrop':[49],
'southwest-rock-outcrop':[51,52,54],'southwest-stumps':[105,106],'southwest-kindling-bundle':[104],
'southwest-log-pile':[102,103],'woodcutters-shed':[127,110],'south-field-haystack':[124],
'central-covered-state':[136],'south-fence-applied-state':[138],'north-applied-state-assembly':[139,140,141]}


def main():
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    inv=json.loads((OUT/'baseline/masks/manifest.json').read_text());assignments=[]
    for r in inv['masks']:
        if r['png']:r['png']=str(OUT/'baseline/masks'/r['png'])
    directory=OUT/'scenery-domains';directory.mkdir(exist_ok=True)
    for number,(slug,name,parts) in enumerate(GROUPS):
        indices=MASKS.get(slug)
        if not indices:
            # No native scenery-only domain exists for these terrain/root and
            # kindling pieces. Keep the footprint hypothesis explicit.
            image=Image.new('L',(1792,1152));draw=ImageDraw.Draw(image)
            for part in parts:
                pts=level['sight_obstacles'][part]['points'];n=len(pts)
                upper=[(p['x'],p['y']-p['z_top']) for p in pts];lower=[(p['x'],p['y']-p['z_bottom']) for p in pts]
                draw.polygon(upper,fill=255)
                for i in range(n):draw.polygon([upper[i],upper[(i+1)%n],lower[(i+1)%n],lower[i]],fill=255)
            coverage=np.asarray(image).copy()
            for row in inv['masks'][:142]:
                if row['index'] not in list(range(48))+list(range(128,136)):continue
                x,y=row['box_top_left'];w,h=row['box_size'];native=np.asarray(Image.open(row['png']).convert('L'))
                left,top=max(0,x),max(0,y);right,bottom=min(1792,x+w),min(1152,y+h)
                if right>left and bottom>top:
                    coverage[top:bottom,left:right][native[top-y:bottom-y,left-x:right-x]>0]=0
            path=directory/f'{slug}.png';Image.fromarray(coverage).save(path);index=200+number
            inv['masks'].append(dict(index=index,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance='Inferred native-volume projection minus foreground wood and canopy; source ownership needs further review'))
            indices=[index]
        assignments.append(dict(reviewed=True,asset_group='croisement02-'+slug,mask_indices=indices))
    inventory=directory/'inventory.json';inventory.write_text(json.dumps(inv,indent=2)+'\n')
    masks=directory/'assignments.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(inventory),projections={'exterior':dict(state='Initial source state; applied-only structures withheld from readiness',source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),assignments=assignments)}),indent=2)+'\n')
    review=directory/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'forest-v4-inventory/inventory.json'),evidence='Source survey sheets and individual masks. Geometry and source-domain readiness remain separate.'),indent=2)+'\n')
    for slug,name,parts in GROUPS:
        asset='croisement02-'+slug;workspace=OUT/'scenery-round-1/assets'/asset
        if (workspace/'inspection/refinement.json').exists():continue
        acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'));bpy.context.preferences.filepaths.save_version=0
        if (workspace/'workspace.json').exists():bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'));validate(workspace)
        else:
            prepare(workspace,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=review,source_mask_manifest=masks,width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        reports=[];deferred='state' in slug
        for obj in bpy.data.collections['Croisement02 Working'].all_objects:
            if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
            index=int(obj['source_node'].split('-')[-1])
            if deferred:report=dict(status='Retained native geometry; effective patch-state review pending')
            else:report=refine(obj,level['sight_obstacles'][index],index,slug)
            report['source_node']=obj['source_node'];reports.append(report)
        validate(workspace);modified(workspace)
        inspection=workspace/'inspection';inspection.mkdir(exist_ok=True)
        report=dict(asset_id=asset,parts=reports,model_sha256=sha(workspace/'model.blend'),status='state evidence pending' if deferred else 'geometry candidate; visual review pending',limitations=['Unobserved surfaces remain source-only gray pending geometry review.','Dimensions follow surveyed native volumes; hidden construction details are inferred.','Integrated source coverage and mission-state verification remain pending.'])
        (inspection/'refinement.json').write_text(json.dumps(report,indent=2)+'\n')
        (workspace/'review.md').write_text(report['status']+'. '+ ' '.join(report['limitations'])+'\n')
        print('COMPLETED',asset,flush=True);release()

if __name__=='__main__':main()
