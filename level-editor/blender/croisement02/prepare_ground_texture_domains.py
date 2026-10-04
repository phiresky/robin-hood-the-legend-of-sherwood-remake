"""Prepare source-preserving ground packets and state reservations, without API use.

These domains are proposals until final receiver geometry and all foreground
ownership are approved. They must not silently delete unmodeled vegetation.
"""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog,bank_workspace,scenery_workspace
from evidence_io import sha,write_json
DEST=OUT/'ground-texture-preparation'


def paste(full,local,x,y):
    h,w=local.shape
    full[max(0,y):min(1152,y+h),max(0,x):min(1792,x+w)] |= local[max(0,-y):min(h,1152-y),max(0,-x):min(w,1792-x)]


def main():
    DEST.mkdir(exist_ok=True)
    bank=bank_workspace('croisement02-north-woodland-bank')
    if bank is None:raise ValueError('Current bank candidate required')
    bank_root=OUT/'terrain-bank-candidate'
    depth=np.load(bank_root/'integration/bank-ground-first-hit.npz')
    names=list(depth['names']);owner=depth['owner'];ground=owner==names.index('ground')
    inventory_path=OUT/'review-mask-inventory.json';inventory=json.loads(inventory_path.read_text())['masks']
    layers_path=OUT/'source-states/layers.json';layers=json.loads(layers_path.read_text())
    lookup={(r.get('layer'),r.get('layer_index')):r['index'] for r in inventory if 'layer_index' in r}
    initial,applied=set(),set()
    for patch in layers['patches']:
        initial.update(lookup[(r['layer'],r['index'])] for r in patch['state']['old_masks'])
        applied.update(lookup[(r['layer'],r['index'])] for r in patch['state']['new_masks'])
    silhouettes={};evidence=[]
    for row in inventory:
        if not row.get('png') or row['index'] in applied-initial:continue
        bitmap=np.zeros((1152,1792),bool);paste(bitmap,np.asarray(Image.open(row['png']).convert('L'))>0,*row['box_top_left'])
        silhouettes[row['index']]=bitmap;evidence.append(dict(index=row['index'],path=row['png'],sha256=sha(row['png'])))
    scenery=np.logical_or.reduce(list(silhouettes.values()))
    animation_path=OUT/'animation-references/manifest.json'
    animations=json.loads(animation_path.read_text())['animations']
    animation=np.zeros((1152,1792),bool);animation_records=[]
    for item in animations:
        frame=item['frames'][0];path=Path(frame['image']);alpha=np.asarray(Image.open(path).convert('RGBA'))[:,:,3]>0
        paste(animation,alpha,*frame['bbox'][:2]);animation_records.append(dict(index=item['index'],kind=item['kind'],profile=item['profile'],path=str(path),sha256=sha(path),bbox=frame['bbox']))
    # Accepted source assignments can include bank/prop artwork without a
    # native silhouette. Reserve them even while their geometry is pending.
    authored=np.zeros_like(scenery);authored_records=[];cache={}
    for group in json.loads(reviewed_catalog().read_text())['groups']:
        if 'wood_mask' in group or group.get('state_only'):continue
        override=OUT/'root-bank-source-revision/domain-370.png'
        if group['id']=='croisement02-northeast-oak-root-bank' and override.exists():
            domain=np.asarray(Image.open(override).convert('L'))>0
            if domain.shape!=(1152,1792):raise ValueError('Root bank source domain dimensions changed')
            authored |= domain
            authored_records.append(dict(asset=group['id'],mask=370,path=str(override),sha256=sha(override),review=str(override.parent/'ownership-review.json'),review_sha256=sha(override.parent/'ownership-review.json'),status='Independently reviewed source toe reserved from ground; geometry approval pending.'))
            continue
        worker=scenery_workspace(group['id']);manifest_path=worker/'source-masks.json'
        if not manifest_path.exists():continue
        manifest=json.loads(manifest_path.read_text());ip=(manifest_path.parent/manifest['mask_inventory']).resolve()
        if ip not in cache:cache[ip]={r['index']:r for r in json.loads(ip.read_text())['masks']}
        for assignment in manifest.get('projections',{}).get('exterior',{}).get('assignments',[]):
            if assignment.get('asset_group')!=group['id']:continue
            for index in assignment.get('mask_indices',[]):
                if index<142:continue
                row=cache[ip][index];path=(ip.parent/row['png']).resolve();a=np.asarray(Image.open(path).convert('L'))>0
                paste(authored,a,*row['box_top_left']);authored_records.append(dict(asset=group['id'],mask=index,path=str(path),sha256=sha(path),manifest=str(manifest_path),manifest_sha256=sha(manifest_path),status='reserved from ground; independent scenery ownership remains required'))
    fence_handoff=OUT/'missing-fence-candidates/v9/handoff.json'
    if fence_handoff.exists():
        for item in json.loads(fence_handoff.read_text())['items']:
            path=Path(item['source_domain_path']);domain=np.asarray(Image.open(path).convert('L'))>0
            if domain.shape!=(1152,1792):raise ValueError('Fence source domain dimensions changed')
            authored |= domain
            authored_records.append(dict(asset=item['id'],mask=item['source_domain'],path=str(path),sha256=sha(path),handoff=str(fence_handoff),handoff_sha256=sha(fence_handoff),status='Pending authored fence source including fallen timber; geometry review remains separate.'))
    scenery |= animation | authored
    known=ground & ~scenery
    source_path=OUT/'animation-references/composite-frame-0.png';source=np.asarray(Image.open(source_path).convert('RGB'))
    rgba=np.dstack((source,known.astype('uint8')*255));rgba[~known,:3]=0
    Image.fromarray(rgba).save(DEST/'ground-observed-source.png')
    Image.fromarray(np.where((ground&~known)[...,None],127,source).astype('uint8')).save(DEST/'ground-fill-input.png')
    Image.fromarray((ground&~known).astype('uint8')*255).save(DEST/'ground-api-edit-mask.png')
    for name,domain in [('ground-first-hit',ground),('ground-observed-domain',known),('ground-hidden-domain',ground&~known),('foreground-exclusion',scenery),('animated-first-frame-exclusion',animation),('authored-scenery-reservation',authored)]:
        Image.fromarray(domain.astype('uint8')*255).save(DEST/f'{name}.png')
    # Every mission frame remains independently addressable. Their union is a
    # reservation, never a claim that all frame pixels belong to terrain.
    states=[];state_union=np.zeros((1152,1792),bool)
    for patch in layers['mission_patches']:
        for state,sequence in patch.get('states',{}).items():
            if not sequence:continue
            for number,frame in enumerate(sequence.get('frames',[])):
                image=OUT/'source-states'/frame['image'];im=Image.open(image).convert('RGBA');alpha=np.asarray(im)[:,:,3]>0
                x,y,w,h=frame['bbox']
                if im.size!=(w,h):raise ValueError('Mission frame dimensions differ from recorded bbox')
                paste(state_union,alpha,x,y)
                states.append(dict(patch=patch['id'],mission=patch['mission'],name=patch['name'],state=state,frame=number,image=str(image),sha256=sha(image),bbox=frame['bbox'],opaque_pixels=int(alpha.sum()),elevation=patch['state']['element_fx']['sprite']['elevation'],integrate_in_background=patch['state'].get('integrate_in_background',False)))
    Image.fromarray(state_union.astype('uint8')*255).save(DEST/'mission-frame-reservations.png')
    # These mask-only ground plants are deliberately retained as unresolved
    # scenery responsibilities. Removing their paint requires replacement
    # geometry/ownership first; this script cannot mark that work complete.
    pending_grass=list(range(111,124))
    grass=np.logical_or.reduce([silhouettes[i] for i in pending_grass])
    Image.fromarray(grass.astype('uint8')*255).save(DEST/'pending-ground-plants.png')
    sheet=Image.new('RGB',(1040,4*220),'#444');draw=ImageDraw.Draw(sheet)
    for n,index in enumerate(pending_grass):
        row=next(r for r in inventory if r['index']==index);x,y=row['box_top_left'];w,h=row['box_size']
        crop=Image.fromarray(source).crop((x,y,x+w,y+h)).convert('RGBA');crop.putalpha(Image.open(row['png']).convert('L'))
        crop.thumbnail((248,188));ox=n%4*260;oy=n//4*220
        draw.text((ox+4,oy+4),f'Pending mask {index}',fill='white');sheet.paste(crop,(ox+6,oy+24),crop)
    sheet.save(DEST/'pending-ground-plants-sheet.png')
    overlay=source.astype(float)*.4
    overlay[known]=source[known]*.6+np.array([0,210,210])*.4
    overlay[ground&scenery]=source[ground&scenery]*.6+np.array([230,20,220])*.4
    overlay[grass]=source[grass]*.4+np.array([255,150,0])*.6
    Image.fromarray(overlay.astype('uint8')).save(DEST/'domain-review.png')
    # Preserve the known pixels exactly, including painted shadows. Unknown
    # domains are placeholders only and are not a generated texture candidate.
    assert np.array_equal(rgba[known,:3],source[known])
    terminal=[r for r in states if 'chariot02_barriere' in r['name'] and r['bbox']==[1018,811,152,152]]
    if terminal:
        row=terminal[0];x,y,w,h=row['bbox'];patch=Image.open(row['image']).convert('RGBA')
        before=Image.fromarray(source).crop((x,y,x+w,y+h)).convert('RGBA')
        after=Image.alpha_composite(before,patch)
        preview=Image.new('RGB',(w*2,h));preview.paste(before,(0,0));preview.paste(after,(w,0));preview.resize((w*6,h*3)).save(DEST/'barrier-state-source-comparison.png')
        barrier_ground_pixels=int(ground[y:y+h,x:x+w].sum())
    else:raise ValueError('Expected barrier state source missing')
    profile_groups={}

    for row in states:
        key=(row['name'],row['state'],row['frame'],row['sha256'],tuple(row['bbox']))
        profile_groups.setdefault(key,[]).append(row['mission'])
    write_json(DEST/'state-source-preservation.json',dict(layers_sha256=sha(layers_path),native_patches=layers['patches'],frames=states,unique_frame_domains=len(profile_groups),reservation_sha256=sha(DEST/'mission-frame-reservations.png'),barrier_ground_requirement=dict(domain=[1018,811,152,152],first_hit_ground_pixels=barrier_ground_pixels,frames=terminal,rule='Apply terminal artwork to real ground receiver in applied state only. Do not flatten the remaining fence or replace base-state ground.')))
    baseline=np.asarray(Image.open(OUT/'baseline/covered.png').convert('RGB'))
    write_json(DEST/'packet.json',dict(version=1,status='preparation only; API not run; terrain geometry and source-domain approval pending',size=[1792,1152],source=str(source_path),source_sha256=sha(source_path),bank_worker=str(bank),bank_model_sha256=sha(bank/'model.blend'),bank_first_hit_sha256=sha(bank_root/'integration/bank-ground-first-hit.npz'),inventory_sha256=sha(inventory_path),catalog_sha256=sha(reviewed_catalog()),mask_evidence=evidence,animated_first_frames=animation_records,animation_manifest_sha256=sha(animation_path),authored_scenery_reservations=authored_records,known_ground_pixels=int(known.sum()),hidden_ground_pixels=int((ground&~known).sum()),state_source_preservation_sha256=sha(DEST/'state-source-preservation.json'),state_frame_records=len(states),pending_ground_plant_masks=pending_grass,pending_ground_plant_pixels=int(grass.sum()),baseline_observed_ground_differences=int(np.any(source!=baseline,axis=2)[known].sum()),known_preservation='Source RGB is copied byte-for-byte wherever ground-observed-domain is255, including painted shadows.',api_edit_mask='ground-api-edit-mask.png:255 editable unseen ground;0 protected observed ground or outside receiver context',api_prompt_draft='Fill only unseen terrain beneath removed scenery. Continue the surrounding forest floor, soil paths and meadow from this same Croisement02 map. Preserve every known pixel and painted ground shadow. Do not paint trees, bushes, logs, fences, buildings or new props into the ground. Supplemental examples must be exact observed ground crops from this map. Return the exact requested1792x1152 canvas.',unresolved=['Animated canopy fringes and first-frame butterflies are excluded using sprite alpha, not just native masks; dynamic animation integration remains separate.','Final receiver first-hit against all actual opaque/foliage geometry is still required; current depth test includes bank and ground only.','Native mask complement may include unmasked props. Source-domain review is pending.','Pending native111–123 ground plants need owned replacement scenery before removal can be integrated.','Approved southwest log-pile source comparison exposed a coherent missing log face; native102/103 remain excluded from ground, pending separate geometry correction.','Other unmodeled shrubs/grass remain explicit excluded scenery, not completed removals.','Bank is geometry candidate only, not user approved. No API request may use this packet as approved terrain yet.','Mission frames include sprites and backgrounds; union is a reservation only, not a terrain ownership assignment.','Applied chariot02_barriere terminal background must be projected on actual ground using its separate state source.'],files={name:sha(DEST/name) for name in ['ground-observed-source.png','ground-fill-input.png','ground-api-edit-mask.png','ground-first-hit.png','ground-observed-domain.png','ground-hidden-domain.png','foreground-exclusion.png','animated-first-frame-exclusion.png','authored-scenery-reservation.png','mission-frame-reservations.png','pending-ground-plants.png','pending-ground-plants-sheet.png','domain-review.png','barrier-state-source-comparison.png']}))
    print('Prepared',int(known.sum()),'known pixels;',len(states),'preserved mission-frame records')


if __name__=='__main__':main()
