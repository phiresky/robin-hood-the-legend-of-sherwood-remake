"""Compare actual physical loop samples with source-order archer compositions."""
import json,sys,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q
ROOT=q.ROOT;DEST=q.OUT/'actual-canopy05-compositions-v1'
SAMPLES=ROOT/'.worktrees/native-editor-delivery/work/canopy05-domain-v1/sampler-packet-v2/browser-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    assert not DEST.exists();DEST.mkdir()
    manifest=json.loads((SAMPLES/'manifest.json').read_text());assert manifest['owner']==dict(kind='map-animation',index=5)
    source=json.loads((q.OUT/'profile02-native-loop-composition-v1/report.json').read_text())
    qualification=json.loads((q.OUT/'qualification.json').read_text());frames=manifest['rows'][0]['frames'];assert len(frames)==len(source['loop_frames'])==14
    pins={str(SAMPLES/'manifest.json'):sha(SAMPLES/'manifest.json')};physical=[]
    for f in frames:
        p=SAMPLES/f['color']['path'];assert sha(p)==f['color']['sha256'];pins[str(p)]=sha(p)
        assert f['render_offset']==[-26,0]
        physical.append(Image.open(p).convert('RGBA'))
    all_records=[]
    for n in [1,2]:
        row=qualification['rows'][n-1]
        crop=json.loads((q.OUT/'profile01-native-loop-composition-v1/report.json').read_text())['crop'] if n==1 else [411,0,641,136]
        w,h=crop[2]-crop[0],crop[3]-crop[1];bg=Image.open(ROOT/qualification['background']['path']).convert('RGBA').crop(crop)
        native_layers=[];physical_layers=[]
        for nf,pf in zip(source['loop_frames'],physical):
            layer=Image.new('RGBA',(w,h));layer.alpha_composite(q.rgba(nf),(nf['top_left'][0]-crop[0],nf['top_left'][1]-crop[1]));native_layers.append(layer)
            layer=Image.new('RGBA',(w,h));layer.alpha_composite(pf,(104-crop[0],-crop[1]));physical_layers.append(layer)
        states=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']
        records=[];sheet=Image.new('RGB',(w*4,(h+30)*4),(22,22,22));draw=ImageDraw.Draw(sheet)
        for j,(phase,f) in enumerate(zip(['initial','transition-0','transition-1','integrated-terminal'],states)):
            patch=Image.new('RGBA',(w,h));patch.alpha_composite(q.rgba(f),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]));mask=np.array(patch)[:,:,3]>0
            per_frame=[]
            for i,(nl,pl) in enumerate(zip(native_layers,physical_layers)):
                nm=np.array(nl)[:,:,3]>0;pm=np.array(pl)[:,:,3]>0
                per_frame.append(dict(frame=i,patch_body_pixels=int(mask.sum()),native_loop_hides_patch=int((mask&nm).sum()),physical_loop_hides_patch=int((mask&pm).sum()),extra_physical_cover_of_native_visible_patch=int((mask&pm&~nm).sum()),missing_physical_cover_of_native_hidden_patch=int((mask&nm&~pm).sum())))
            base=Image.alpha_composite(bg,patch)
            pairs=[Image.alpha_composite(base,native_layers[0]),Image.alpha_composite(base,physical_layers[0]),Image.alpha_composite(base,native_layers[7]),Image.alpha_composite(base,physical_layers[7])]
            for k,(im,label) in enumerate(zip(pairs,['Native loop f0 front','Actual physical f0 front','Native loop f7 front','Actual physical f7 front'])):
                sheet.paste(im.convert('RGB'),(k*w,j*(h+30)+30));draw.text((k*w+3,j*(h+30)+3),phase,fill='white');draw.text((k*w+3,j*(h+30)+16),label,fill='white')
            records.append(dict(phase=phase,source=f['image'],frames=per_frame))
        sheet.save(DEST/f'profile{n:02}-loop-foreground.png')
        if n==1:
            sheet=Image.new('RGB',(w*4,(h+30)*3),(22,22,22));draw=ImageDraw.Draw(sheet)
            for j,f in enumerate(states[:3]):
                patch=Image.new('RGBA',(w,h));patch.alpha_composite(q.rgba(f),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]))
                images=[Image.alpha_composite(Image.alpha_composite(bg,l),patch) for l in [native_layers[0],physical_layers[0],native_layers[7],physical_layers[7]]]
                for k,im in enumerate(images):sheet.paste(im.convert('RGB'),(k*w,j*(h+30)+30));draw.text((k*w+3,j*(h+30)+3),['initial','transition-0','transition-1'][j]+' / '+['native0','physical0','native7','physical7'][k],fill='white')
            sheet.save(DEST/'profile01-patch-foreground.png')
        all_records.append(dict(profile=n,crop=crop,records=records))
    report=dict(status='ACTUAL_SAMPLED_RGBA_SOURCE_ORDER_COMPARISON_STATIC_OWNERSHIP_NOT_YET_QUALIFIED',sample_manifest_sha256=sha(SAMPLES/'manifest.json'),pins=pins,profiles=all_records,placement=dict(native_raster_top_left=[104,0],anchor_minus_center=[130,0],render_offset=[-26,0]),limits=['Exact actual sampler RGBA is composed without native-alpha clipping.','Native static background is retained for this diagnostic; this is not a baked final receiver.','Physical-only occlusion must be split into legitimate static tree contributions and moving source-owner contributions before approval.','All Archer01/02 occurrences are zero-elevation base FX, drawn in the background before elevated canopy05; the patch-foreground sheet is a counterfactual diagnostic only.','Display-polyline minima do not override zero-elevation background routing.','No source geometry, material, ownership or canonical assets modified.'])
    (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([dict(profile=r['profile'],states=[dict(phase=s['phase'],extra_min=min(f['extra_physical_cover_of_native_visible_patch'] for f in s['frames']),extra_max=max(f['extra_physical_cover_of_native_visible_patch'] for f in s['frames'])) for s in r['records']]) for r in all_records]),flush=True)
if __name__=='__main__':main()
