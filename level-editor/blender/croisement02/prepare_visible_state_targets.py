"""Preserve mission scenery targets separately from invisible patch metadata."""
import hashlib,json,math,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from catalog import OUT,ROOT
DATA=ROOT/'datadirs/fullgame_gog_hackable/Data'
BANKS={'Trapcr02','chariot02','chariot03','TG_BowTarget','TG_Panel'}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    dest=OUT/'state-target-evidence';dest.mkdir(exist_ok=True);profiles={};instances=[]
    for mission in sorted((OUT/'state-candidate-v1/missions').glob('*.rhm.json')):
        data=json.loads(mission.read_text())
        for index,target in enumerate(data['targets']):
            if target['filename'] not in BANKS:continue
            key=(target['filename'],target['profile_name'])
            identifier=f"{key[0]}-{len(profiles):02d}"
            if key not in profiles:
                bank=DATA/'Animations/Day'/f'{key[0]}.rhs.d';manifest=bank/'manifest.json'
                profile=next(p for p in json.loads(manifest.read_text())['profiles'] if p['name']==key[1])
                folder=dest/'profiles'/identifier;folder.mkdir(parents=True,exist_ok=True);rows=[]
                for row in profile['rows']:
                    frames=[]
                    for number,frame in enumerate(row['frames']):
                        source=bank/profile['name']/row['path']/frame['file']
                        if not source.exists():source=bank/row['path']/frame['file']
                        raw=folder/'raw'/str(row['action_id'])/str(row['direction'])/frame['file'];raw.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,raw)
                        rgba=np.array(Image.open(source).convert('RGBA'));rgba[np.all(rgba[:,:,:3]==[0,251,0],axis=2),3]=0
                        output=folder/f"action-{row['action_id']}-direction-{row['direction']}-frame-{number:03d}.png";Image.fromarray(rgba).save(output)
                        frames.append(dict(index=number,image=str(output),image_sha256=sha(output),raw=str(raw),raw_sha256=sha(raw),offset=[frame['offset_x']-profile['center_x'],frame['offset_y']-profile['center_y']],size=[rgba.shape[1],rgba.shape[0]],delay=frame['delay'],ticks=frame['delay']+1,duration_seconds=(frame['delay']+1)/25,sound_id=frame['sound_id'],opaque_pixels=int((rgba[:,:,3]>0).sum())))
                    rows.append(dict(action_id=row['action_id'],action=row['action'],direction=row['direction'],action_done=row['action_done'],frames=frames,duration_seconds=sum(f['duration_seconds'] for f in frames)))
                profilepath=folder/'native-profile.json';profilepath.write_text(json.dumps(profile,indent=2)+'\n')
                profiles[key]=dict(id=identifier,bank=key[0],profile=key[1],manifest_sha256=sha(manifest),profile_file=str(profilepath),profile_sha256=sha(profilepath),rows=rows)
            instances.append(dict(mission=mission.stem.removesuffix('.rhm'),mission_sha256=sha(mission),target_index=index,profile_id=profiles[key]['id'],target=target))
    records=list(profiles.values());sheet=Image.new('RGB',(1000,len(records)*190),'#333333');draw=ImageDraw.Draw(sheet)
    for index,p in enumerate(records):
        instance=next(t for t in instances if t['profile_id']==p['id']);draw.text((5,index*190+2),p['profile'],fill='white')
        choices=[]
        for row in p['rows']:
            if row['direction']==0:
                choices.extend([(row,0)] if len(row['frames'])==1 else [(row,0),(row,len(row['frames'])//2),(row,len(row['frames'])-1)])
        for column,(row,frameindex) in enumerate(choices[:5]):
            frame=row['frames'][frameindex];im=Image.open(frame['image']).convert('RGBA');im.thumbnail((190,150));x=column*200;y=index*190+25;sheet.paste(im,(x,y),im);draw.text((x,y+152),f"action{row['action_id']} frame{frameindex}",fill='white')
    sheet.save(dest/'target-profiles.png')
    report=dict(status='source evidence; no geometry completion claim',profiles=records,instances=instances,timing=dict(ticks_per_second=25,ticks_per_frame='serialized delay + 1'),metadata_group_associations=[dict(group='croisement02-central-covered-state',patch=5,obstacles=[142,143],target_profiles=['Croisement02 - buches01_b1','Croisement02 - buches01_b2','Croisement02 - buches01_b3'],shadow_profile='Croisement02 - buches_ombre'),dict(group='croisement02-west-covered-state',patch=6,obstacles=[144],target_profiles=['Croisement02 - rocher01_b1'],shadow_profile='Croisement02 - rocher01_ombre'),dict(group='croisement02-south-fence-applied-state',patch=7,obstacles=[145],target_profiles=['chariot02','chariot02_b2','chariot02_b3']),dict(group='croisement02-north-applied-state-assembly',patch=8,obstacles=[146,147,148,149],target_profiles=['chariot03','chariot03_b2'])],association_status='Position/profile association; exact native script links are reviewed separately. Metadata is invisible, target appearances are visible and require distinct candidate geometry.')
    (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(len(records),'profiles',len(instances),'instances')
if __name__=='__main__':main()
