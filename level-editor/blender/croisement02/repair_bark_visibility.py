"""Review-visible bark uses native foreground exclusions and wood self-occlusion."""
import argparse
import json
import sys
import uuid
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from bark_materials import fill
from tree_geometry import wood_geometry,replace_mesh
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import validate,modified,_render
from revise_feedback import geometry
from audit_candidates import audit
from render_tree import render_workspace


def revise(mask,redo=False):
    w=OUT/f'forest-v4-round-1/assets/croisement02-tree-{mask:02}';receipt=w/'inspection/visible-bark-revision.json'
    acquire()
    if receipt.exists() and redo:receipt.rename(receipt.with_name('visible-bark-revision-archive-'+uuid.uuid4().hex[:8]+'.json'))
    if not receipt.exists():
        bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(w)
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
        before=sha(w/'model.blend');before_geometry=geometry(objects)
        report=json.loads((w/'inspection/refinement.json').read_text())
        if mask==0:
            paths=next(r['paths'] for r in json.loads((OUT/'wood-traces.json').read_text()) if r['mask']==0)
            wood=sorted([o for o in objects if o.get('projection_component')!='crown'],key=lambda o:o['source_node'])
            if [o['source_node'] for o in wood]!=['building-044','building-045'] or len(paths)!=2:raise ValueError('Unexpected trunk assignment')
            # The right source trace is part 044; the left is part 045. The
            # source Y average is unsuitable for comparing different trunk heights.
            paths=sorted(paths,key=lambda p:np.mean(np.asarray(p)[:,0]),reverse=True)
            level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());reports=[]
            for obj,path in zip(wood,paths):
                native=level['sight_obstacles'][int(obj['source_node'].split('-')[-1])]
                ground=float(np.mean([p['y'] for p in native['points']]))
                vertices,faces=wood_geometry([path],ground)
                r=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials));r['source_node']=obj['source_node'];reports.append(r)
                for face in obj.data.polygons:face.use_smooth=len(face.vertices)==4
            report['wood']=reports
            modified(w)
        cfg=json.loads((w/'workspace.json').read_text());path=Path(cfg['source_mask_manifest']);m=json.loads(path.read_text());write_json(w/'inspection/before-visible-bark-source-masks.json',m)
        inv_path=Path(m['mask_inventory']);inv=json.loads(inv_path.read_text());rows={r['index']:r for r in inv['masks']}
        def canvas(index):
            row=rows[index];im=Image.open((inv_path.parent/row['png']).resolve()).convert('L');x,y=row['box_top_left'];a=np.asarray(im)>0
            out=np.zeros((1152,1792),bool);left,top=max(0,x),max(0,y);right,bottom=min(1792,x+im.width),min(1152,y+im.height)
            if right>left and bottom>top:out[top:bottom,left:right]=a[top-y:bottom-y,left-x:right-x]
            return out
        domain=canvas(mask);exclusions=[i for i in [*range(54,94),*range(128,136)] if np.any(domain&canvas(i))]
        assignment=next(a for a in m['projections']['exterior']['assignments'] if a.get('asset_group')==w.name)
        assignment.update(exclude_mask_indices=exclusions,exclusions_reviewed=True,exclusion_reason='Reviewed visible wood uses native wood coverage minus foreground shrub and canopy alpha. Coarse neighboring proxy volumes cannot establish visible bark ownership.')
        write_json(path,m)
        report['bark']=fill(w,objects,mask,receiver_only=True,donor_mapping='aperiodic-vertical')
        stage=w/('.modified-visible-bark-'+uuid.uuid4().hex[:8]);_render(cfg,stage,w/'input/views.json');validate(w)
        (w/'history').mkdir(exist_ok=True);(w/'modified').rename(w/'history'/stage.name);stage.rename(w/'modified')
        bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'));report['model_sha256']=sha(w/'model.blend')
        report['limitations'].append('Visible bark projection uses wood self-occlusion with native canopy/shrub exclusions; coarse context proxies are excluded. Rear bark and source-occluded wood remain inferred.')
        write_json(w/'inspection/refinement.json',report);audit(w)
        write_json(receipt,dict(before_model_sha256=before,model_sha256=report['model_sha256'],before_geometry_sha256=before_geometry,geometry_sha256=geometry(objects),excluded_masks=exclusions,ownership_report=str(w/'inspection/bark-ownership.json')))
    elif json.loads(receipt.read_text())['model_sha256']!=sha(w/'model.blend'):raise ValueError('Revised worker changed')
    evidence=[w/'inspection/actual-materials/evidence.json',w/'inspection/source-coverage/report.json']
    if not all(p.exists() and json.loads(p.read_text()).get('model_sha256')==sha(w/'model.blend') for p in evidence):render_workspace(w,256,release_slot=False)
    release()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('masks',nargs='+',type=int);parser.add_argument('--redo',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    if not set(args.masks)<={0,5,27,41,46}:raise ValueError('Only the source-reviewed bark candidates are supported')
    for n in args.masks:
        if latest.get(f'croisement02-tree-{n:02}',{}).get('decision')=='approved':raise ValueError('Approved worker is frozen')
    try:
        for n in args.masks:revise(n,args.redo)
    finally:release()

if __name__=='__main__':main()
