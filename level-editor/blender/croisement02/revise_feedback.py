"""Apply reviewed source-specific corrections and rebuild fixed-camera evidence."""
import argparse
import json
import sys
import uuid
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json,digest
from render_slots import acquire,release
from refinement_workspace import validate,modified,_render
from source_projection_bake import bake
from feedback_geometry import wall,gate,firewood,kindling,wattle
from audit_candidates import audit
from render_tree import render_workspace


def geometry(objects):
    return digest([dict(name=o.name,transform=[list(r) for r in o.matrix_world],vertices=[list(v.co) for v in o.data.vertices],faces=[list(p.vertices) for p in o.data.polygons]) for o in sorted(objects,key=lambda o:o.name)])


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--assets',nargs='+',required=True);parser.add_argument('--redo',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());inventory=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    def canvas(index):
        r=next(r for r in inventory['masks'] if r['index']==index);im=Image.open(OUT/'baseline/masks'/r['png']).convert('L');out=Image.new('L',(1792,1152));out.paste(im,tuple(r['box_top_left']));return np.asarray(out)>0
    for slug in args.assets:
        w=OUT/('scenery-round-2' if slug in ('north-kindling-bundle','south-field-wattle-fence') else 'scenery-round-1')/'assets'/('croisement02-'+slug);receipt=w/'inspection/feedback-revision-1.json'
        if receipt.exists():
            if not args.redo:continue
            receipt.rename(receipt.with_name('feedback-revision-1-archive-'+uuid.uuid4().hex[:8]+'.json'))
        acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(w)
        objects=sorted([o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name],key=lambda o:o['source_node'])
        old_hash=sha(w/'model.blend');before=geometry(objects);notes=[]
        if slug=='east-stone-wall-and-gate':
            mask=canvas(101)
            for o in objects:
                i=int(o['source_node'].split('-')[-1])
                if i!=9:wall(o,level['sight_obstacles'][i],mask)
            notes.append('Wall crest follows native source-mask height instead of mean-height rectangular blocks.')
        elif slug=='southeast-stone-wall-and-gate':
            o=next(o for o in objects if o['source_node']=='building-012');gate(o,level['sight_obstacles'][12])
            for o in objects:
                i=int(o['source_node'].split('-')[-1])
                if i!=12:wall(o,level['sight_obstacles'][i],canvas(96)|canvas(97))
            notes.append('Gate rebuilt with two square posts, three rails and the source-facing rising diagonal.')
        elif slug=='north-firewood-stack':
            firewood(objects,canvas(109));notes.append('One continuous three-tier pile across both canonical parts; shorter logs point back from the visible cut ends.')
        elif slug=='south-field-wattle-fence':
            for o in objects:wattle(o,level['sight_obstacles'][int(o['source_node'].split('-')[-1])])
            notes.append('Visible post heights and sagging weave profile traced from artwork; source domain restores visible weave absent from native actor-occlusion coverage.')
        elif slug=='north-kindling-bundle':
            kindling(objects);notes.append('One leaning bundle across both canonical parts instead of two separate cones.')
        if slug in ('east-stone-wall-and-gate','southeast-stone-wall-and-gate','east-rail-fence'):
            cfg=json.loads((w/'workspace.json').read_text());path=Path(cfg['source_mask_manifest']);m=json.loads(path.read_text())
            assignment=next(a for a in m['projections']['exterior']['assignments'] if a.get('asset_group')==w.name)
            domain=np.logical_or.reduce([canvas(i) for i in assignment['mask_indices']])
            overlaps=[i for i in range(128,136) if np.any(domain&canvas(i))]
            if overlaps:
                assignment['exclude_mask_indices']=sorted(set(assignment.get('exclude_mask_indices',[])+overlaps));assignment['exclusions_reviewed']=True;assignment['exclusion_reason']='Native foreground foliage overlapping the surveyed masonry/fence artwork must not be baked onto its solid surfaces.'
                write_json(path,m)
        changed=geometry(objects)!=before
        if changed:modified(w)
        # Native/scanned source domains establish visible artwork ownership here.
        # Coarse neighbouring crown proxies do not establish pixel visibility.
        cfg=json.loads((w/'workspace.json').read_text());target_nodes=[o['source_node'] for o in objects]
        report_path=w/'inspection/feedback-source-ownership.json'
        ownership=bake('Croisement02',cfg['source_path'],report_path,receiver_nodes=target_nodes,occluder_nodes=target_nodes,projection_label='exterior',elevation_deg=35,preserve_authored=False,source_mask_manifest=cfg['source_mask_manifest'])
        notes.append('Source rays use receiver self-occlusion within reviewed artwork domains; unrelated coarse context proxies are excluded. Hidden surfaces remain unknown gray.')
        stage=w/('.modified-feedback-'+uuid.uuid4().hex[:8]);_render(cfg,stage,w/'input/views.json');validate(w)
        (w/'history').mkdir(exist_ok=True);(w/'modified').rename(w/'history'/stage.name);stage.rename(w/'modified')
        bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'));write_json(w/'validation.json',validate(w))
        r=json.loads((w/'inspection/refinement.json').read_text());r['model_sha256']=sha(w/'model.blend');r['limitations']+=notes;r['feedback_revision']=1;write_json(w/'inspection/refinement.json',r)
        write_json(receipt,dict(before_model_sha256=old_hash,model_sha256=r['model_sha256'],before_geometry_sha256=before,geometry_sha256=geometry(objects),geometry_changed=changed,known_texels=ownership['known_texels'],unknown_texels=ownership['unknown_texels'],notes=notes))
        audit(w);render_workspace(w,256,release_slot=False);release();print('REVISED',slug,flush=True)

if __name__=='__main__':main()
