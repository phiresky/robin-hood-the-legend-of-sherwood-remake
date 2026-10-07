"""Review a private native-camera sign compositor without editing scene assets."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
BASE=OUT/'restart10-physical-signs'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return {'file':str(p),'sha256':sha(p)}
def main():
 dst=BASE/'ordering-review-v2';dst.mkdir(exist_ok=False)
 browser=BASE/'ordering-browser-v1';proof=json.loads((browser/'verification.json').read_text())
 manifest=json.loads((BASE/'input-v2/manifest.json').read_text())
 reference=OUT/'state-sign-candidate/native-order-reference-v3/manifest.json'
 records=json.loads(reference.read_text())['records']
 ambient_file=OUT/'animation-references/manifest.json';animations={a['index']:a for a in json.loads(ambient_file.read_text())['animations']}
 imagepins={};results=[];opposite=[];verified_images={}
 for r in records:
  target=r['target_index'];instance=next(x for x in manifest['instances']if x['target_index']==target);t=instance['native_target']
  overlays=[]
  for a in r['overlapping_animations']:
   assert a['after_sign'];frames=animations[a['index']]['frames'];overlays=frames
  cycles=max(len(overlays),1);selected=[];samples=[]
  for phase in range(32):
   name=f'target-{target}-phase-{phase}-native'
   sign=Image.open(browser/(name+'-sign.png')).convert('RGBA');static=Image.open(browser/(name+'-static.png')).convert('RGBA')
   ordered=Image.alpha_composite(static,sign);signal=np.asarray(sign)[:,:,3]
   # Evaluate every independent ambient phase, not an assumed shared wall clock.
   for ambient in range(cycles):
    overlay=Image.new('RGBA',(512,512))
    if overlays:
     f=overlays[ambient];p=Path(f['image'])
     if str(p) not in verified_images:
      sourcepath=Path(f['source']);assert sha(sourcepath)==f['sha256']
      raw=np.asarray(Image.open(sourcepath).convert('RGBA')).copy();raw[np.all(raw[:,:,:3]==[0,251,0],axis=2)]=0
      source=Image.open(p).convert('RGBA');assert np.array_equal(raw,np.asarray(source))
      imagepins[str(p)]=sha(p);imagepins[str(sourcepath)]=sha(sourcepath);verified_images[str(p)]=source
     x,y,w,h=f['bbox'];source=verified_images[str(p)];assert source.size==(w,h)
     # The review camera spans160 projected source units and is centered16 world units above the anchor.
     offsetx=t['position_x']-80-x;offsety=t['position_y']-16*math.cos(math.radians(35))-80-y
     overlay=source.transform((512,512),Image.Transform.AFFINE,(1/3.2,0,offsetx,0,1/3.2,offsety),resample=Image.Resampling.NEAREST)
    rgba=np.asarray(overlay).copy();rawalpha=rgba[:,:,3].copy()
    rgba[:,:,3]=((rawalpha.astype('uint16')*signal.astype('uint16')+127)//255).astype('uint8')
    composite=Image.alpha_composite(ordered,Image.fromarray(rgba))
    pixels=np.asarray(composite);outside=signal==0
    assert np.array_equal(pixels[outside],np.asarray(static)[outside]),'Escaped physical sign footprint'
    visible=(signal==255)&(rawalpha==0)
    assert np.array_equal(pixels[visible],np.asarray(sign)[visible]),'Static receiver hides native-visible physical sign'
    samples.append({'phase':phase,'ambient_phase':ambient,'uncovered_opaque_sign_render_pixels':int(visible.sum()),'ambient_intersecting_sign_render_pixels':int(((rawalpha>0)&(signal>0)).sum()),'outside_sign_unchanged':True})
    if ambient==(phase//2)%cycles:
     f=dst/(name+'-ordered.png');composite.convert('RGB').save(f);selected.append(f)
   if phase in (0,8,16,24):
    f=browser/f'target-{target}-phase-{phase}-opposite.png';old=BASE/'browser-v5'/f.name
    a=np.asarray(Image.open(f));b=np.asarray(Image.open(old));assert np.array_equal(a,b),'Oblique physical rendering changed'
    opposite.append({'target':target,'phase':phase,'current':pin(f),'prior':pin(old),'changed_channels':0})
  sheet=Image.new('RGB',(4*384,3*410),(45,45,45));draw=ImageDraw.Draw(sheet)
  for j,phase in enumerate((0,8,16,24)):
   paths=[BASE/'browser-v5'/f'target-{target}-phase-{phase}-native.png',selected[phase],browser/f'target-{target}-phase-{phase}-opposite.png']
   for row,p in enumerate(paths):
    sheet.paste(Image.open(p).convert('RGB').resize((384,384)),(j*384,row*410+26));draw.text((j*384+5,row*410+6),f'{["Native physical depth", "Native ordered + source ambient", "Opposite unchanged physical"][row]} | pose {phase}',fill='white')
  f=dst/f'target-{target}-ordering12.png';sheet.save(f)
  results.append({'target':target,'phase_cases':len(samples),'samples':samples,'sheet':pin(f),'selected_frames':[pin(p)for p in selected]})
 report={'status':'PASS private compositor invariants; visual/root review pending; not installed runtime', 'physical_model':manifest['model'],'source_reference':pin(reference),'ambient_manifest':pin(ambient_file),'render_proof':pin(browser/'verification.json'),'results':results,'opposite_preservation':opposite,'ambient_resources':imagepins,
 'behavior':['Target drawing does not request static silhouette masking. Static art precedes target rendering.','Polyline animation order uses minimum polyline Y and per-element map-position side tests. Non-polyline target action coordinates determine insertion after display coordinates establish physical placement.','Source-order compatibility is restricted to the exact35-degree native orthographic camera. The private fixture uses isolated original physical sign renders, with internal depth retained.','Later native ambient RGBA is composited only within the physical sign render footprint; independent ambient phases are exhaustively enumerated.'],
 'limitations':['Private proof is a hybrid camera-specific compatibility experiment, not a shared renderer implementation or full physical ambient animation.','Compositing projected ambient color inside the physical sign footprint does not establish physical wind/flutter geometry.','Actual phase timing remains independent; illustrative sheets select an explicitly controlled phase only.','Other actors and source shadow-key blending are outside this narrow proof. Existing painted physical shadow is retained.','Neighbor geometry and textures remain untouched. Earlier static occlusion diagnostics do not prove neighbor geometry defects.','No user approval or live publication is implied.']}
 (dst/'report.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({'targets':len(results),'phase_cases':sum(r['phase_cases']for r in results),'opposite_matches':len(opposite),'report':pin(dst/'report.json')}))
if __name__=='__main__':main()
