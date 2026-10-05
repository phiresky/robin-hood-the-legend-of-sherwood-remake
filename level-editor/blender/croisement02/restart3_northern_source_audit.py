"""Classify fixed northern scene gaps against native target, shadow and surface evidence."""
import json,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
DEST=OUT/'restart3-northern-source-audit'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    DEST.mkdir(exist_ok=True);inputs=[]
    def read(p):inputs.append(p);return json.loads(p.read_text())
    proof=OUT/'restart3-scene-audit/coherent-batch-v3-v1';audit=read(proof/'first-hit/audit.json');inventory=read(OUT/'forest-v4-mask-inventory.json');size=(1792,1152);masks={};maskrows={}
    for i in [3,5,6,133,136,137]:
        row=next(x for x in inventory['masks']if x['index']==i);p=Path(row['png']);inputs.append(p);canvas=Image.new('L',size);canvas.paste(Image.open(p),tuple(row['box_top_left']));masks[str(i)]=np.asarray(canvas)>0;maskrows[str(i)]={k:row.get(k)for k in ['index','layer_index','box_top_left','box_size','mask_type']}
    bank=OUT/'terrain-bank-candidate/bank-source-domain.png';inputs.append(bank);masks['bank']=np.asarray(Image.open(bank))>0
    anim=next(a for a in read(OUT/'animation-references/manifest.json')['animations']if a['index']==5);phases=[]
    for f in anim['frames']:
        p=Path(f['image']);inputs.append(p);canvas=Image.new('RGBA',size);canvas.alpha_composite(Image.open(p).convert('RGBA'),tuple(f['bbox'][:2]));phases.append(np.asarray(canvas)[:,:,3]>0)
    masks['canopy_any']=np.logical_or.reduce(phases);states={}
    for kind in ['log-trap','rock-trap']:
        data=read(OUT/f'state-target-evidence/{kind}/manifest.json');body=Image.new('RGBA',size);shadow=Image.new('RGBA',size)
        for part in data['parts']:
            f=part['initial'];p=Path(f['image']);inputs.append(p);assert sha(p)==f['image_sha256'];body.alpha_composite(Image.open(p).convert('RGBA'),tuple(int(part['position'][i]+f['offset'][i])for i in [0,1]))
        for binding in data['background_bindings']:
            f=binding['states']['initial']['frames'][0];p=OUT/'source-states'/f['image'];inputs.append(p);shadow.alpha_composite(Image.open(p).convert('RGBA'),tuple(f['bbox'][:2]))
        masks[kind]=np.asarray(body)[:,:,3]>0;masks[kind+'-shadow']=np.asarray(shadow)[:,:,3]>0
        states[kind]=dict(mission=data['mission'],metadata_patch=data['native_metadata_patch'],body_profiles=[p['profile']for p in data['parts']],background_bindings=[b['id']for b in data['background_bindings']])
    source=OUT/'baseline/covered.png';inputs.append(source);native=Image.open(source).convert('RGB');records=[]
    for c in audit['components']:
        if c['component']not in [1,5,8]:continue
        idx=c['component'];perpixel=[];counts=Counter();over=np.array(native);xy=np.array([s['pixel']for s in c['samples']]);assert not masks['canopy_any'][xy[:,1],xy[:,0]].any();assert not masks['133'][xy[:,1],xy[:,0]].any()
        for s in c['samples']:
            x,y=s['pixel'];roles=[k for k,m in masks.items()if m[y,x]]
            if any(masks[k][y,x]for k in ['log-trap','rock-trap']):role='initial_target_body';color=(240,140,60)
            elif any(masks[k][y,x]for k in ['log-trap-shadow','rock-trap-shadow']):role='initial_shadow_only';color=(180,100,240)
            elif any(masks[k][y,x]for k in ['3','5','6']):role='wood_domain_residual';color=(255,60,70)
            elif masks['bank'][y,x]:role='bank_domain_residual';color=(70,150,255)
            else:role='metadata_only_residual';color=(255,240,70)
            counts[role]+=1;perpixel.append(dict(pixel=[x,y],classification=role,overlapping_authorities=roles,current_first_hit=s['asset'],current_world_hit=s['hit']));over[y,x]=color
        files={}
        for role in counts:
            canvas=np.zeros((1152,1792),np.uint8)
            for p in perpixel:
                if p['classification']==role:canvas[p['pixel'][1],p['pixel'][0]]=255
            path=DEST/f'region-{idx}-{role}.png';Image.fromarray(canvas).save(path);files[role]=dict(path=str(path),sha256=sha(path),pixels=counts[role])
        box=c['bounds'];box=(box[0]-10,box[1]-10,box[2]+10,box[3]+10);im=native.crop(box).resize(((box[2]-box[0])*6,(box[3]-box[1])*6),Image.Resampling.NEAREST);marked=Image.fromarray(over).crop(box).resize(im.size,Image.Resampling.NEAREST);sheet=Image.new('RGB',(im.width*2,im.height+42),(35,35,35));sheet.paste(im,(0,0));sheet.paste(marked,(im.width,0));d=ImageDraw.Draw(sheet);d.text((4,im.height+2),'Native / orange initial body; purple shadow; red wood-domain; blue bank; yellow metadata-only',fill='white');d.text((4,im.height+20),f'Region{idx}: metadata overlap is not a physical surface assignment',fill='white');sheet.save(DEST/f'region-{idx}-state-partition.png')
        records.append(dict(region=idx,bounds=c['bounds'],samples=len(perpixel),current_first_hits=c['first_hits'],counts=dict(counts),zero_native_canopy_pixels=True,classification_masks=files,pixels=perpixel))
    actions={1:dict(owner='state_completion',action='Integrate native initial log body2542 and shadow-only14 at their exact mission identity. Remaining8 are5bank and3wood06edge samples; inspect as bounded perimeter, not new crown.'),5:dict(owner='state_completion with terrain_domains/remaining_geometry residual survey',action='Integrate native initial rock body500 and shadow-only108. Remaining329 are295wood03-domain,3bank and31metadata-only; native crop contains dark ground vegetation/root context, so do not convert the entire wood mask to new solid wood. Preserve source underlay for mission variants and survey only these residual samples.'),8:dict(owner='remaining_geometry tree06 wood; terrain_domains bank',action='313 samples are native tree06 wood domain and visually continuous bark/root; propose private bounded root/bank contact correction preserving native projection/RGBA, existing crown and neighboring ground. All current rays hit bank nearZ43.949. Remaining131 bank-owned samples show grass/underlay: restore native bank appearance separately, without inventing shadow geometry. Actual hidden root positions and feasible contact interval must be inspected before choosing vertex movement.')}
    report=dict(status='Read-only northern ownership diagnosis complete; no geometry changes',scene_sha256=audit['scene_sha256'],sources={str(p):sha(p)for p in dict.fromkeys(inputs)},native_mask_identity=maskrows,state_bindings=states,canopy_phases_checked=len(phases),regions=records,proposed_actions=actions,conclusion='Regions1/5 are predominantly missing mission initial bodies/shadow presentation in a static-only snapshot, not missing crowns. Region8 is a distinct tree06 root/bank appearance/contact defect. Metadata masks alone are not ownership proof.',limitations=['These3945 fixed diagnostic coordinates are not a whole-map completeness census.','Shadow masks record exact source alpha presence, not physical shadow volume or light reconstruction.','Global mask inventory136/137 have layer_index135/136; preserve both identities when reconciling native patch references.','Native wood masks can include ground vegetation/root shadow. Region5 residual295 remains semantically uncertain despite mask3 overlap.','Source ray corrections require separate evaluated hidden-geometry/contact evidence; this audit launches no Blender and proposes no exact displacement.','No source masks, canonical catalog, geometry, materials or approved receipts modified.'])
    (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(DEST/'report.json')
if __name__=='__main__':main()
