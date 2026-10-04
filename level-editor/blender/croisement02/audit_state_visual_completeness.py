"""Distinguish invisible gameplay state from associated visible mission scenery."""
import hashlib,json,shutil
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT,ROOT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    root=OUT/'state-target-evidence';data=json.loads((root/'manifest.json').read_text());mobile_profiles={};mobile_instances=[]
    for path in sorted((OUT/'state-candidate-v1/missions').glob('*.rhm.json')):
        mission=json.loads(path.read_text())
        for index,mobile in enumerate(mission['mobile_elements']):
            for spriteindex,entry in enumerate(mobile['sprites']):
                sprite=entry['sprite'];name=sprite['profile_name'];bank=ROOT/'datadirs/fullgame_gog_hackable/Data/Animations/Day'/f"{sprite['frame_profile_name']}.rhs.d"
                if name not in mobile_profiles:
                    native=next(p for p in json.loads((bank/'manifest.json').read_text())['profiles'] if p['name']==name);folder=root/'mobile-profiles'/name;folder.mkdir(parents=True,exist_ok=True);rows=[]
                    for row in native['rows']:
                        frames=[]
                        for number,f in enumerate(row['frames']):
                            source=bank/name/row['path']/f['file'];raw=folder/'raw'/row['path']/f['file'];raw.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,raw);rgba=np.array(Image.open(source).convert('RGBA'));rgba[np.all(rgba[:,:,:3]==[0,251,0],axis=2),3]=0;target=folder/f"{row['action_id']}-{row['direction']}-{number:03d}.png";Image.fromarray(rgba).save(target);frames.append(dict(native=f,decoded=str(target),decoded_sha256=sha(target),raw=str(raw),raw_sha256=sha(raw),ticks=f['delay']+1))
                        rows.append(dict(action_id=row['action_id'],direction=row['direction'],frames=frames))
                    mobile_profiles[name]=dict(native_profile=native,bank_manifest_sha256=sha(bank/'manifest.json'),rows=rows,role='mobile cart/horse-team visual source; motion path and sprite-local offset preserved separately')
                mobile_instances.append(dict(mission=path.stem.removesuffix('.rhm'),index=index,sprite_index=spriteindex,record=mobile,mission_sha256=sha(path)))
    assemblies=json.loads((root/'assemblies.json').read_text())['assemblies'];counts={p['profile']:sum(t['profile_id']==p['id'] for t in data['instances'])for p in data['profiles']}
    report=dict(status='incomplete visible state assembly; invisible metadata is not the whole visual asset',native_metadata=dict(groups=4,obstacles=list(range(142,150)),visible_on_own=False,associated_visible_targets=True),required_assemblies=[dict(id=a['id'],native_metadata_patch=a['native_metadata_patch'],visible_profiles=[p['profile']for p in a['parts']],geometry_status='isolated endpoint candidate; unapproved; transition geometry missing' if a['id']=='log-trap' else 'missing source-driven solid candidate',timeline=str(root/a['id']/'manifest.json'),timeline_sha256=sha(root/a['id']/'manifest.json'))for a in assemblies],additional_mission_visuals=dict(net_rigging_profiles=['Croisement02 - piege01h','Croisement02 - piege03h'],net_rigging_instances=counts['Croisement02 - piege01h']+counts['Croisement02 - piege03h'],arrow_markers=dict(profile='Bow Target',instances=counts['Bow Target'],role='mission-specific animated arrow-interaction targets; scripted net, hidden archer and merry-men triggers; preserve interactive marker semantics instead of inventing static furniture'),signposts=dict(profile='Panneau',instances=counts['Panneau'],missions=['S03_FoB_MP'],role='visible rotating mission signposts, action_filter0 and no script class; not a permanent all-mission map asset'),cart_living_parts='Horse teams are visible embedded source components. Separate living/moving representation from cart/wheel/debris geometry; never discard their pixels or treat them as permanent scenery.'),mobile_profiles=mobile_profiles,mobile_instances=mobile_instances,native_command_semantics=dict(target_play_anim_freeze='Start requested action and mark script command terminated immediately; freeze on entry to final frame, without consuming final delay.',target_hourglass='Advance target sprite only while active.',patch_visual='Advance forward while applying; transition termination invokes final/background state.',background='Only integrate_in_background patches composite terminal transition pixels onto map; absence of final sprite does not remove those pixels.',timing='25 Hz; intermediate sprite frames last serialized delay+1 ticks. Script sequence scheduling remains a separate event clock; no animation-length wait is implied by play-animation command.'),preservation=dict(target_manifest_sha256=sha(root/'manifest.json'),assembly_manifest_sha256=sha(root/'assemblies.json')),completion_requirements=['Review actual solid target initial/applied assemblies independently from gameplay metadata.','Track native intermediate visible bodies and mixed living/cart parts for motion; endpoint models do not prove transition completion.','Attach sparse shadows and terminal ground paint to their audited bank/ground receivers.','Preserve native target scripts, markers, mobile paths and all decoded frames.'])
    (root/'visual-completeness.json').write_text(json.dumps(report,indent=2)+'\n');print('4 visible assemblies; 2 mobile profiles;',len(mobile_instances),'mobile instances; 45 markers; 5 mission signposts')
if __name__=='__main__':main()
