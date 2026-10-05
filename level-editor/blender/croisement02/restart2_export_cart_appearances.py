"""Preserve separately timed cart target artwork as explicit planar appearance clips."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from restart2_native_appearance_glb import build

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    for assembly in ['south-cart','north-cart']:
        src=OUT/'state-target-evidence'/assembly/'manifest.json';m=json.loads(src.read_text());dest=OUT/'restart2-state'/f'{assembly}-source-v1';dest.mkdir(exist_ok=False);box=m['bbox'];size=(box[2]-box[0],box[3]-box[1]);records=[];previous=None;sounds=[];bound=set()
        for part in m['parts']:
            t=part['start_tick']
            for f in [part['initial']]+part['frames']:
                if f['image']not in bound:
                    image=Path(f['image']);assert sha(image)==f['image_sha256'];a=np.array(Image.open(image).convert('RGBA'));assert not np.any(np.all(a[:,:,:3]==[0,0,255],axis=2)&(a[:,:,3]>0));bound.add(f['image'])
            for f in part['frames']:
                if f['sound_id']:sounds.append(dict(tick=t,target_index=part['target_index'],frame=f['index'],sound_id=f['sound_id']))
                t+=f['ticks']
        for tick in range(m['terminal_geometry_reached_tick']+1):
            canvas=Image.new('RGBA',size);selected=[]
            for part in m['parts']:
                clock=part['start_tick'];frame=part['initial']
                if tick>=clock:
                    for frame in part['frames']:
                        clock+=frame['ticks']
                        if tick<clock:break
                x,y=[int(math.floor(a+b+.5))for a,b in zip(part['position'],frame['offset'])];canvas.alpha_composite(Image.open(frame['image']).convert('RGBA'),(x-box[0],y-box[1]));selected.append(dict(target_index=part['target_index'],profile_id=part['profile_id'],frame=frame['index'],source_sha256=frame['image_sha256']))
            digest=hashlib.sha256(canvas.tobytes()).hexdigest()
            if digest==previous:records[-1]['last_tick']=tick;continue
            filename=f'{tick:03}.png';canvas.save(dest/filename);records.append(dict(first_tick=tick,last_tick=tick,image=filename,rgba_sha256=digest,native_frames=selected));previous=digest
        report=dict(tick_rate=25,terminal_tick=m['terminal_geometry_reached_tick'],records=records,source_manifest_sha256=sha(src),mission=m['mission'],bbox=box,target_instances=[{k:p[k]for k in ['profile_id','target_index','script_class','position','start_tick','terminal_frame_reached_tick','freeze_final_frame']}for p in m['parts']],sound_events=sounds,metadata_swap_tick=m['metadata_swap_tick'],native_metadata_patch=m['native_metadata_patch'],background_bindings=m['background_bindings'],scope='Exact separately timed target artwork only; not recovered rigid-body motion.',limitations=['Mobile horse-team actors before and after target playback remain separately addressed mission actors.','Background patch, sound playback and metadata swaps are preserved as bindings but not performed by this standalone GLB.','Physical cart solids, shadows and full-scene layer composition remain independent requirements.'])
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');build(assembly,source=dest);print(assembly,len(records),'phases',len(sounds),'sound bindings')
if __name__=='__main__':main()
