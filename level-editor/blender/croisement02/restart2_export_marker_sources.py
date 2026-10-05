"""Export marker actions and all script-addressed instances with separate reserved shadow masks."""
import json,hashlib,zipfile
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from restart2_native_appearance_glb import build


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source=OUT/'state-target-evidence/manifest.json';data=json.loads(source.read_text());profile=next(p for p in data['profiles']if p['id']=='TG_BowTarget-02');instances=[i for i in data['instances']if i['profile_id']==profile['id']];assert len(instances)==45
    call_path=OUT/'net-state-source-review-v1/script-call-bindings.json';calls=json.loads(call_path.read_text())['markers'];assert len(calls)==45
    dest=OUT/'restart2-state/marker-source-export-v1';dest.mkdir(exist_ok=False);body_dest=OUT/'restart2-state/marker-body-source-v1';body_dest.mkdir(exist_ok=False)
    all_frames=[f for r in profile['rows']for f in r['frames']];left=min(f['offset'][0]for f in all_frames);top=min(f['offset'][1]for f in all_frames);right=max(f['offset'][0]+f['size'][0]for f in all_frames);bottom=max(f['offset'][1]+f['size'][1]for f in all_frames);size=(int(right-left),int(bottom-top));rows=[];body_records=[]
    for row in profile['rows']:
        tick=0;frames=[]
        for f in row['frames']:
            image=Path(f['image']);raw=Path(f['raw']);assert sha(image)==f['image_sha256']and sha(raw)==f['raw_sha256'];pixels=np.array(Image.open(image).convert('RGBA'));key=np.all(pixels[:,:,:3]==[0,0,255],axis=2)&(pixels[:,:,3]>0);body=pixels.copy();body[key,3]=0;mask=np.zeros_like(pixels);mask[key]=[255,255,255,255]
            assert int((body[:,:,3]>0).sum())+int(key.sum())==f['opaque_pixels'];offset=(int(f['offset'][0]-left),int(f['offset'][1]-top));canvas=Image.new('RGBA',size);canvas.alpha_composite(Image.fromarray(body),offset);shadow=Image.new('RGBA',size);shadow.alpha_composite(Image.fromarray(mask),offset);name=f'action-{row["action_id"]}-frame-{f["index"]:03}';(dest/(name+'-raw.png')).write_bytes(raw.read_bytes());canvas.save(dest/(name+'-body.png'));shadow.save(dest/(name+'-shadow-mask.png'))
            frame=dict(index=f['index'],first_tick=tick,last_tick=tick+f['ticks']-1,ticks=f['ticks'],sound_id=f['sound_id'],offset=f['offset'],size=f['size'],raw_image=name+'-raw.png',raw_sha256=sha(raw),body_image=name+'-body.png',body_sha256=sha(dest/(name+'-body.png')),shadow_mask=name+'-shadow-mask.png',shadow_mask_sha256=sha(dest/(name+'-shadow-mask.png')),shadow_pixels=int(key.sum()));frames.append(frame)
            if row['action_id']==0:
                filename=f'{f["index"]:03}.png';canvas.save(body_dest/filename);body_records.append(dict(first_tick=tick,last_tick=tick+f['ticks']-1,image=filename,rgba_sha256=hashlib.sha256(canvas.tobytes()).hexdigest(),source_image_sha256=f['image_sha256']))
            else:assert not np.any(pixels[:,:,3]),'Hide action is not transparent'
            tick+=f['ticks']
        rows.append(dict(action_id=row['action_id'],action=row['action'],direction=row['direction'],cycle_ticks=tick,loop=row['action_id']==0,frames=frames))
    assert len(body_records)==20 and body_records[-1]['last_tick']==59
    bindings=[]
    for instance in instances:
        call=next(c for c in calls if(c['mission'],c['target_index'])==(instance['mission'],instance['target_index']));bindings.append(dict(instance=instance,script_binding=call))
    report=dict(status='PASS source/action/instance export; runtime presentation remains separate',source_manifest_sha256=sha(source),script_bindings_sha256=sha(call_path),profile_sha256=profile['profile_sha256'],native_hz=25,canvas_bounds=[left,top,right,bottom],actions=rows,instances=bindings,shadow_contract=dict(reserved_rgb=[0,0,255],native_key_rgb565=31,ordinary_shadow_level=40,fog_shadow_level=10,operation='Preserve the shadow mask as destination darkening, not blue paint or a fixed-colored texture.',presentation='Body-only GLB is accompanied by exact per-phase shadow masks. Production native-layer composition must apply the active ambience.'),limitations=['Body-only browser playback does not include reserved shadow-key composition.','No static archery furniture is created.','Editor mission instancing and script-triggered action visibility remain integration requirements.'])
    (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    manifest=dict(tick_rate=25,terminal_tick=59,cycle_ticks=60,records=body_records,scope='Marker color body only; exact reserved shadow masks remain in companion source export')
    (body_dest/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');build('marker-body',source=body_dest,loop=True)
    archive=dest/'marker-source-export.zip'
    with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED)as z:
        for path in sorted(dest.iterdir()):
            if path.suffix in ['.png','.json']:z.write(path,path.name)
    with zipfile.ZipFile(archive)as z:
        for name in z.namelist():assert hashlib.sha256(z.read(name)).hexdigest()==sha(dest/name)
    (dest/'archive-verification.json').write_text(json.dumps(dict(status='PASS',archive_sha256=sha(archive),instances=45,actions=[r['action_id']for r in rows],frame_records=sum(len(r['frames'])for r in rows),all_exported_files_roundtrip_exact=True),indent=2)+'\n');print('Exported45 marker identities,20 loop phases and2 blank action aliases')


if __name__=='__main__':main()
