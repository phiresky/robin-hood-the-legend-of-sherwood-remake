"""Correct wall-return identification and exclude surveyed foreground shrubs."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from scenery_geometry import refine
from refinement_workspace import validate,modified
from render_slots import acquire,release
from evidence_io import sha


def main():
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    for workspace in sorted((OUT/'scenery-round-1/assets').iterdir()):
        receipt=workspace/'inspection/domain-review.json'
        if receipt.exists():continue
        config=json.loads((workspace/'workspace.json').read_text());manifest_path=Path(config['source_mask_manifest']);manifest=json.loads(manifest_path.read_text())
        inv_path=Path(manifest['mask_inventory']);inv=json.loads(inv_path.read_text());rows={r['index']:r for r in inv['masks']}
        def canvas(index):
            row=rows[index];image=Image.open((inv_path.parent/row['png']).resolve()).convert('L');x,y=row['box_top_left'];w,h=image.size
            out=np.zeros((1152,1792),bool);left,top=max(0,x),max(0,y);right,bottom=min(1792,x+w),min(1152,y+h)
            if right>left and bottom>top:out[top:bottom,left:right]=np.asarray(image)[top-y:bottom-y,left-x:right-x]>0
            return out
        assignment=next(r for r in manifest['projections']['exterior']['assignments'] if r.get('asset_group')==workspace.name)
        if workspace.name.endswith('southwest-rock-outcrop'):assignment['mask_indices']=[51,52]
        domain=np.logical_or.reduce([canvas(i) for i in assignment['mask_indices']]);exclusions=[i for i in range(54,94) if np.any(domain&canvas(i))]
        repair_wall=workspace.name.endswith('east-rail-fence')
        if exclusions:
            assignment.update(exclude_mask_indices=exclusions,exclusions_reviewed=True,exclusion_reason='Foreground underbrush silhouettes inspected in native-mask sheets; their pixels must not be projected onto masonry, wood or terrain receivers.')
        acquire();bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(workspace)
        manifest_path.write_text(json.dumps(manifest,indent=2)+'\n')
        notes=[]
        if repair_wall:
            for obj in bpy.data.collections['Croisement02 Working'].all_objects:
                if obj.type=='MESH' and obj.get('asset_group')==workspace.name:
                    index=int(obj['source_node'].split('-')[-1]);refine(obj,level['sight_obstacles'][index],index,'southeast-stone-wall-and-gate');obj['asset_name']='Southeast Stone Wall Returns'
            notes.append('Source detail confirms parts 018/024 are masonry wall returns; former rail-fence label is retained only as a stable legacy identifier.')
        if exclusions or repair_wall or workspace.name.endswith('southwest-rock-outcrop'):
            validate(workspace);modified(workspace)
        receipt.write_text(json.dumps(dict(model_sha256=sha(workspace/'model.blend'),foreground_shrub_exclusions=exclusions,notes=notes),indent=2)+'\n')
        path=workspace/'inspection/refinement.json';report=json.loads(path.read_text());report['model_sha256']=sha(workspace/'model.blend');report['domain_review']=str(receipt)
        if repair_wall:report['limitations'].append(notes[0])
        path.write_text(json.dumps(report,indent=2)+'\n');print('REVIEWED',workspace.name,flush=True);release()

if __name__=='__main__':main()
