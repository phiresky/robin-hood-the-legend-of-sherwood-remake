"""Record complete native source scope and the boundaries requiring refinement."""
import hashlib
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement'

def main():
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    masks=json.loads((OUT/'mask-survey/index.json').read_text())['masks']
    for row in masks:
        i=row['index']
        if i<=28:kind='wood; visible leaf/terrain overlaps need exclusion'
        elif 29<=i<=85:kind='vegetation; mixed stems/rocks require ownership split'
        elif 86<=i<=109:kind='rock/cliff; foreground vegetation requires exclusion'
        elif 110<=i<=112:kind='bridge rails/support; authored geometry absent from obstacle scene'
        elif i==113:kind='stone wall; foreground leaf exclusion pending'
        elif i==114:kind='firewood stack'
        elif i==115:kind='stream rock'
        elif 116<=i<=124:kind='animated crown occupancy; static and animated RGB remain distinct'
        else:kind='state/terrain domain; initial visual ownership pending state audit'
        row['classification']=kind
        row['review_state']='source visually inspected; receiver ownership unfinished'
    animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations']
    for row in animations:
        row['kind']='canopy' if row['index'] in range(8) or row['index']==10 else 'moving water' if row['index'] in [8,9] else 'ambient creature'
    layers=json.loads((OUT/'source-states/layers.json').read_text())
    result=dict(map='Croisement03',status='refinement in progress; no user-approved candidates',source_sha256=hashlib.sha256((OUT/'baseline/covered.png').read_bytes()).hexdigest(),native_parts=106,baseline_meshes=107,mask_count=len(masks),layers=sorted({r['layer'] for r in masks}),masks=masks,animations=animations,native_patch_count=len(level['patches']),mission_patch_count=len(layers['mission_patches']),source_review=dict(depth_layers=[0,1],obstacle_sheets=list(range(4)),mask_sheets=list(range(9)),notes=['Source map is 1408 by 960 pixels.','Native tree masks16/17/27 have no matching obstacle part and need supplemental wood.','Timber bridge deck, handrails, diagonal supports and stream-crossing fallen log need explicit authored owners.','Layer1 duplicates do not create a second physical tree or shrub.','North and side boundary crowns/rocks must be completed beyond the artwork edge.','Ground, moving water, mission objects and 101 mission patches remain separate completion obligations.']),requirements=['Only Leicester southeast cottage tree and Leicester moat bank tree are allowed supplementary tree assets.','Do not mistake the gameplay occupancy silhouette for a full opaque animation image.','Source artwork may show foliage over a native wood/rock mask; do not project the whole union onto wood/rock.','Geometry approval and texture approval are separate hash-bound decisions.'])
    (OUT/'source-inventory.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ['native_parts','mask_count','layers','native_patch_count','mission_patch_count']}))
if __name__=='__main__':main()
