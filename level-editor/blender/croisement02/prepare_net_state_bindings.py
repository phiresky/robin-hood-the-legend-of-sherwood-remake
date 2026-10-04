"""Bind idle target removal to net variants, ground remnants and final animation loops."""
import json
from pathlib import Path
from PIL import Image,ImageDraw
from catalog import OUT
from native_log_foreground_reference import sha

def main():
    root=OUT/'state-target-evidence';layers=OUT/'source-states/layers.json';data=json.loads(layers.read_text());targets=json.loads((root/'manifest.json').read_text());dest=OUT/'net-state-bindings-v1';dest.mkdir(exist_ok=False);assemblies=[]
    for number in ['01','03']:
        idle=next(p for p in targets['profiles']if p['profile']==f'Croisement02 - piege{number}h');instances=[t for t in targets['instances']if t['profile_id']==idle['id']];patches=[p for p in data['mission_patches']if p['name']in[f'Croisement02 - piege{number}{suffix}'for suffix in ['e','i','g']]];records=[]
        for p in patches:
            states={}
            for name,state in p['states'].items():
                clock=0;frames=[]
                for frame in state['frames']:
                    path=OUT/'source-states'/frame['image'];ticks=frame['delay']+1;frames.append(dict(**frame,source_sha256=sha(path),first_tick=clock,last_tick=clock+ticks-1));clock+=ticks
                states[name]=dict(action_id=state['action_id'],nominal_cycle_ticks=clock,loop_after_transition=name=='final',frames=frames)
            records.append(dict(mission=p['mission'],runtime_patch_index=p['runtime_patch_index'],name=p['name'],integrate_in_background=p['state']['integrate_in_background'],states=states,source_state=p['state']))
        variants=[]
        for suffix in ['e','i','g']:
            record=next(p for p in records if p['name'].endswith(number+suffix));frames=record['states']['transition']['frames'];final=record['states']['final']['frames'];selected=[frames[0],frames[len(frames)//2],frames[-1],final[0]];sheet=Image.new('RGB',(960,300),'#222');draw=ImageDraw.Draw(sheet)
            for i,frame in enumerate(selected):
                image=Image.open(OUT/'source-states'/frame['image']).convert('RGBA');image.thumbnail((220,260));sheet.paste(image,(i*240+10,30),image);draw.text((i*240+10,8),['transition start','transition middle','transition end','final loop phase0'][i],fill='white')
            path=dest/f'net-{number}-{suffix}-source.png';sheet.save(path);variants.append(dict(suffix=suffix,source_sheet=path.name,transition_frames=len(frames),final_frames=len(final),final_cycle_ticks=record['states']['final']['nominal_cycle_ticks']))
        assemblies.append(dict(id=f'net-{number}',idle_profile=idle,mission_instances=instances,unique_positions=sorted({(t['target']['position_x'],t['target']['position_y'])for t in instances}),patches=records,variants=variants,script_operation='Arrow interaction switches idle target to transparent action160, updates animated marker, applies one e/i branch together with g patch. Occupied branch also relocates/locks selected actors as scripted.',geometry_status='missing reviewed source-supported net, rope and ground-remnant geometry'))
    result=dict(status='source-bound assembly contract; target disappearance alone is not complete net motion',layers_sha256=sha(layers),target_manifest_sha256=sha(root/'manifest.json'),tick_rate=25,assemblies=assemblies,semantics=['Patch final action remains active and advances with default looping progression; retain every14frame final net phase.','During transition, completion invokes applied final state; reversing an applied transition uses reversed progression.','All listed net patches have integrate_in_background false; final images remain active state sprites, not permanent background paint.','Target action160 is a visibility change, not the whole lifting animation.'],limitations=['Variants and actor occupancy require mission script binding, not one permanent all-mission trap.','No geometry, actor replacement, scene modification or runtime implementation is claimed.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print([(a['id'],len(a['mission_instances']),len(a['patches']),a['variants'])for a in assemblies])
if __name__=='__main__':main()
