"""Stage separately timed visible mission assemblies and their gameplay swaps."""
import hashlib,json,math
from pathlib import Path
from PIL import Image,ImageDraw
from catalog import OUT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    root=OUT/'state-target-evidence';source=json.loads((root/'manifest.json').read_text());profiles={p['profile']:p for p in source['profiles']};allrecords=[]
    definitions=[('log-trap','Emb05_FoB_MP',['Croisement02 - buches01_b1','Croisement02 - buches01_b2','Croisement02 - buches01_b3'],[0,0,0],5,45,'Croisement02 - buches_ombre'),('rock-trap','Emb05_FoB_MP',['Croisement02 - rocher01_b1'],[0],6,60,'Croisement02 - rocher01_ombre'),('south-cart','Emb05_FoB_MP',['chariot02','chariot02_b2','chariot02_b3'],[0,100,133],7,100,'chariot02_barriere'),('north-cart','Emb09_FoB_JMS',['chariot03','chariot03_b2'],[0,100],8,100,None)]
    patches=json.loads((OUT/'source-states/layers.json').read_text())['mission_patches'];background=Image.open(OUT/'baseline/covered.png').convert('RGBA')
    for identifier,mission,names,starts,patch,swap,shadow in definitions:
        folder=root/identifier;folder.mkdir(exist_ok=True);parts=[];bounds=[]
        for name,start in zip(names,starts):
            profile=profiles[name];target=next(t for t in source['instances'] if t['mission']==mission and t['profile_id']==profile['id'])
            row=next(r for r in profile['rows'] if r['action_id']==160 and r['direction']==0);initial=next(r for r in profile['rows'] if r['action_id']==0 and r['direction']==0)['frames'][0]
            record=dict(profile_id=profile['id'],profile=name,target_index=target['target_index'],script_class=target['target']['script_class'],position=[target['target']['position_x'],target['target']['position_y']],start_tick=start,terminal_frame_reached_tick=start+sum(f['ticks'] for f in row['frames'][:-1]),frames=row['frames'],initial=initial,freeze_final_frame=True)
            for frame in [initial]+row['frames']:
                if not frame['opaque_pixels']:continue
                x,y=[int(math.floor(a+b+.5))for a,b in zip(record['position'],frame['offset'])];w,h=frame['size'];bounds.append((x,y,x+w,y+h))
            parts.append(record)
        box=[min(b[0] for b in bounds)-12,min(b[1] for b in bounds)-12,max(b[2] for b in bounds)+12,max(b[3] for b in bounds)+12];duration=max(p['start_tick']+sum(f['ticks']for f in p['frames'])for p in parts)
        ticks=sorted(set([-1,0,duration-1,swap]+[round(duration*i/6)for i in range(1,6)]));sheet=Image.new('RGB',(380*3,280*math.ceil(len(ticks)/3)),'#333333');draw=ImageDraw.Draw(sheet);samples=[]
        for sample,tick in enumerate(ticks):
            canvas=Image.new('RGBA',(box[2]-box[0],box[3]-box[1]));selected=[]
            for part in parts:
                frame=part['initial']
                if tick>=part['start_tick']:
                    counter=part['start_tick']
                    for frame in part['frames']:
                        counter+=frame['ticks']
                        if tick<counter:break
                x,y=[int(math.floor(a+b+.5))for a,b in zip(part['position'],frame['offset'])];canvas.alpha_composite(Image.open(frame['image']).convert('RGBA'),(x-box[0],y-box[1]));selected.append(dict(profile=part['profile'],frame=frame['index'],image_sha256=frame['image_sha256']))
            path=folder/f'tick-{tick:03d}.png';canvas.save(path);context=background.crop(box);context.alpha_composite(canvas);context.save(folder/f'tick-{tick:03d}-context.png');preview=context.copy();preview.thumbnail((380,250));left=(sample%3)*380;top=(sample//3)*280;sheet.paste(preview,(left,top+25));draw.text((left+5,top+5),'initial'if tick<0 else f'tick {tick} / {tick/25:.2f}s',fill='white');samples.append(dict(tick=tick,image=str(path),sha256=sha(path),selected_frames=selected))
        sheet.save(folder/'timeline.png')
        shadowbindings=[dict(id=p['id'],mission=p['mission'],runtime_patch_index=p['runtime_patch_index'],states=p['states'],integrate_in_background=p['state']['integrate_in_background'])for p in patches if p['name']==shadow]
        record=dict(id=identifier,mission=mission,parts=parts,bbox=box,nominal_row_duration_ticks=duration,terminal_geometry_reached_tick=max(p['terminal_frame_reached_tick'] for p in parts),native_metadata_patch=patch,metadata_swap_tick=swap,metadata_invisible=True,visible_target_geometry_status='missing distinct 3D candidate',background_patch_profile=shadow,shadow_start_semantics='Next script sequence step immediately after starting target animations, not after animation completion' if identifier in ('log-trap','rock-trap') else None,background_bindings=shadowbindings,samples=samples,script_evidence=dict(path=str(root/'scripts'/f'{mission}.ts'),sha256=sha(root/'scripts'/f'{mission}.ts')),timing_note='25 ticks/s; each sprite frame lasts delay+1 ticks; action160 parts retain separate starts and freeze at their own terminal frame. Relative startup timing excludes player-dismissed popup duration. Target play-animation commands finish immediately after starting; frozen targets stop on entry to final frame, without consuming its delay.',limitations=['Ground shadows and terminal ground painting are bound separately and not included as geometry in this target-only preview.','Carts include horses and moving debris; a complete solid target model requires per-part tracking, not extruding metadata obstacles.'])
        (folder/'manifest.json').write_text(json.dumps(record,indent=2)+'\n');allrecords.append(record)
    (root/'assemblies.json').write_text(json.dumps(dict(assemblies=allrecords),indent=2)+'\n')
    print([(r['id'],r['nominal_row_duration_ticks'],r['bbox'])for r in allrecords])
if __name__=='__main__':main()
