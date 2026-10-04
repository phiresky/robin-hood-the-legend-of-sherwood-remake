"""Bind a controlled native foliage phase and target draw-order reference."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def crop_frame(frame,box):
    x,y,w,h=frame['bbox'];canvas=Image.new('RGBA',(box[2]-box[0],box[3]-box[1]));canvas.alpha_composite(Image.open(frame['image']).convert('RGBA'),(x-box[0],y-box[1]));return canvas

def main():
    root=OUT/'state-target-evidence/log-trap';m=json.loads((root/'manifest.json').read_text());alltargets=json.loads((OUT/'state-target-evidence/manifest.json').read_text());part=m['parts'][-1];target=next(r['target']for r in alltargets['instances']if r['mission']==m['mission']and r['target_index']==part['target_index']);key=min(p[1]for p in target['polyline']);animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];box=m['bbox'];native=Image.open(root/'tick-089.png').convert('RGBA');wood=np.array(native)[:,:,3]>0;base=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop(box);dest=root/'native-order-reference';dest.mkdir(exist_ok=True);rows=[];phase_results=[]
    relevant=[]
    for record in animations:
        frame=record['frames'][0];x,y,w,h=frame['bbox'];overlap=x<box[2]and x+w>box[0]and y<box[3]and y+h>box[1]
        if not overlap:continue
        poly=record['display_polyline'];assert poly,'Overlapping no-polyline animation needs explicit non-animation merge handling'
        order=min(p[1]for p in poly);relevant.append((order,record));rows.append(dict(animation=record['index'],profile=record['profile'],display_polyline=poly,sort_key=order,relative_order='after target'if order>key else'before target',phase0_image=frame['image'],phase0_sha256=sha(Path(frame['image'])),bbox=frame['bbox']))
    assert all(order!=key for order,_ in relevant),'Equal display keys need tie-order evidence'
    for phase in range(14):
        canvas=base.copy();foreground=np.zeros_like(wood)
        for order,record in sorted(relevant,key=lambda p:p[0]):
            if order<key:canvas.alpha_composite(crop_frame(record['frames'][phase%len(record['frames'])],box))
        canvas.alpha_composite(native)
        for order,record in sorted(relevant,key=lambda p:p[0]):
            if order>key:
                overlay=crop_frame(record['frames'][phase%len(record['frames'])],box);pixels=np.array(overlay);assert not np.any((pixels[:,:,3]>0)&np.all(pixels[:,:,:3]==0,axis=2)),'Shadow-key pixels require separate blend handling';foreground|=pixels[:,:,3]>0;canvas.alpha_composite(overlay)
        visible=wood&~foreground;phase_results.append(dict(phase=phase,raw_target_pixels=int(wood.sum()),covered_by_later_animation=int((wood&foreground).sum()),visible_target_pixels=int(visible.sum())))
        if phase==0:
            canvas.save(dest/'native-composite-phase0.png');Image.fromarray(visible.astype('uint8')*255).save(dest/'visible-log-phase0.png');Image.fromarray(foreground.astype('uint8')*255).save(dest/'later-canopy-phase0.png');base.save(dest/'static-baseline.png')
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'];mask=next(x for x in masks if x['index']==130);maskpath=OUT/'baseline/masks'/mask['png'];a=np.array(Image.open(maskpath).convert('L'))>0;canopy=animations[2]['frames'][0];b=np.array(Image.open(canopy['image']))[:,:,3]>0;mx,my=mask['box_top_left'];x,y,w,h=canopy['bbox'];p=np.zeros_like(a);p[y-my:y-my+h,x-mx:x-mx+w]=b
    packet_code=Path(__file__).with_name('tree_geometry.py');code_hash=sha(packet_code);snapshot=dest/f'observed-construction-{code_hash}.py'
    if not snapshot.exists():snapshot.write_bytes(packet_code.read_bytes())
    comparison=dict(observed_packet_code_snapshot=str(snapshot),projectile_mask_sha256=sha(maskpath),native_phase0_sha256=sha(Path(canopy['image'])),mask_pixels=int(a.sum()),animated_phase_pixels=int(p.sum()),mask_outside_animated_phase=int((a&~p).sum()),animated_phase_outside_mask=int((p&~a).sum()),animated_alpha_parities={f'{dy},{dx}':int(p[dy::2,dx::2].sum())for dy in range(2)for dx in range(2)},interpretation='Occupancy-versus-animated-overlay comparison only. Static baseline foliage/photo support remains and these counts alone do not prove wrong canopy visual alpha.',packet_construction='tree_geometry.foliage_packet uses native mask alpha with composite-frame-0 RGB; complete static-plus-overlay source must be considered.',packet_code_sha256=code_hash,static_baseline_sha256=sha(OUT/'baseline/covered.png'),static_plus_overlay_sha256=sha(OUT/'animation-references/composite-frame-0.png'))
    report=dict(status='Controlled source reference, not a gameplay screenshot or state approval',target_profile=part['profile'],target_polyline=target['polyline'],target_sort_key=key,target_rgba_sha256=sha(root/'tick-089.png'),animations=rows,phase_results=phase_results,canopy_source_comparison=comparison,limitations=['Phase0 is controlled to match static worker appearance; actual event start has an independent canopy animation phase.','Target FX does not request runtime sprite masking. Source transparency and later visual-animation RGBA are distinct.','Static background remains beneath target/animation draws; projectile occupancy is not imposed as target clipping.','Native animated foliage has checkerboard occupancy. The holes reveal whatever was drawn earlier; they are not a claim that full physical crowns have50%missing surfaces.','Ground shadow transitions are preserved separately and not composited in this bounded wood visibility reference.']);(dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(phase_results[0],comparison)
if __name__=='__main__':main()
