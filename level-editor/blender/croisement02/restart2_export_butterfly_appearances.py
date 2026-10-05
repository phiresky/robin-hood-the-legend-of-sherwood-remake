"""Bind all seven native butterfly effects to reusable timed appearance exports."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from restart2_native_appearance_glb import build


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source=OUT/'animation-references/manifest.json';all_animations=json.loads(source.read_text())['animations'];effects=[a for a in all_animations if 'papillon'in a['profile']];assert len(effects)==7
    bindings=[]
    for effect in effects:
        name=f'butterfly-{effect["index"]:02}';dest=OUT/'restart2-state'/f'{name}-source-v1';dest.mkdir(exist_ok=False)
        frames=effect['frames'];left=min(f['bbox'][0]for f in frames);top=min(f['bbox'][1]for f in frames);right=max(f['bbox'][0]+f['bbox'][2]for f in frames);bottom=max(f['bbox'][1]+f['bbox'][3]for f in frames);tick=0;records=[]
        for index,frame in enumerate(frames):
            raw=Path(frame['source']);assert sha(raw)==frame['sha256'];path=Path(frame['image']);image=Image.open(path).convert('RGBA');pixels=np.asarray(image);assert not np.any(np.all(pixels[:,:,:3]==[0,0,255],axis=2)&(pixels[:,:,3]>0));canvas=Image.new('RGBA',(right-left,bottom-top));canvas.alpha_composite(image,(frame['bbox'][0]-left,frame['bbox'][1]-top));filename=f'{index:03}.png';canvas.save(dest/filename);duration=frame['delay']+1
            records.append(dict(first_tick=tick,last_tick=tick+duration-1,image=filename,rgba_sha256=hashlib.sha256(canvas.tobytes()).hexdigest(),source_image_sha256=sha(path),raw_sha256=frame['sha256'],source_bbox=frame['bbox']));tick+=duration
        assert len(frames)==99 and tick==198
        manifest=dict(status='Exact native butterfly source/timing binding',tick_rate=25,terminal_tick=tick-1,cycle_ticks=tick,records=records,native_index=effect['index'],native_profile=effect['profile'],sprite=effect['sprite'],display_polyline=effect['display_polyline'],screen_bounds=[left,top,right,bottom],anchor_in_canvas=[effect['sprite']['position_x']-left,effect['sprite']['position_y']-top],source_manifest_sha256=sha(source),limitations=['Effect artwork and movement within the sprite are preserved; no anatomical 3D butterfly reconstruction is claimed.','Library placement must retain the original sprite anchor, elevation and native ordering.'])
        (dest/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');build(name,source=dest,loop=True);bindings.append(dict(id=name,native_index=effect['index'],profile=effect['profile'],source_manifest_sha256=sha(dest/'manifest.json'),export_manifest=str(OUT/'restart2-state'/f'{name}-native-appearance-v1/manifest.json')))
    (OUT/'restart2-state/butterfly-export-bindings-v1.json').write_text(json.dumps(dict(status='Seven native effect exports; browser verification and full scene integration pending',effects=bindings),indent=2)+'\n')


if __name__=='__main__':main()
