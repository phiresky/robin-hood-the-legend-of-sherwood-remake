"""Separate cart-source color keys from visible bodies without losing source pixels."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from native_log_foreground_reference import sha

def main():
    root=OUT/'state-target-evidence';dest=root/'cart-key-domains-v1';dest.mkdir(exist_ok=False);targets=json.loads((root/'manifest.json').read_text());visual=json.loads((root/'visual-completeness.json').read_text());records=[]
    inputs=[]
    for profile in targets['profiles']:
        if not profile['profile'].startswith('chariot'):continue
        for row in profile['rows']:
            for frame in row['frames']:inputs.append((profile['id'],f"target-{row['action_id']}-{row['direction']}-{frame['index']:03d}",Path(frame['image']),dict(kind='target',action=row['action_id'],direction=row['direction'],frame=frame['index'],offset=frame['offset'],ticks=frame['ticks'])))
    for name,profile in visual['mobile_profiles'].items():
        for row in profile['rows']:
            for index,frame in enumerate(row['frames']):inputs.append((name,f"mobile-{row['action_id']}-{row['direction']}-{index:03d}",Path(frame['decoded']),dict(kind='mobile-horse-team',action=row['action_id'],direction=row['direction'],frame=index,native=frame['native'],ticks=frame['ticks'])))
    for group,name,path,binding in inputs:
        source=np.array(Image.open(path).convert('RGBA'));opaque=source[:,:,3]>0;shadow=opaque&np.all(source[:,:,:3]==[0,0,255],axis=2);body=opaque&~shadow;assert not(body&shadow).any();assert np.array_equal(body|shadow,opaque)
        folder=dest/group;folder.mkdir(exist_ok=True);p=folder/f'{name}-body.png';q=folder/f'{name}-shadow-key.png';Image.fromarray(body.astype(np.uint8)*255).save(p);Image.fromarray(shadow.astype(np.uint8)*255).save(q);records.append(dict(source=str(path),source_sha256=sha(path),binding=binding,body_pixels=int(body.sum()),shadow_key_pixels=int(shadow.sum()),body_domain=str(p.relative_to(dest)),shadow_key_domain=str(q.relative_to(dest)),body_domain_sha256=sha(p),shadow_domain_sha256=sha(q)))
    result=dict(status='source component masks only; actor/cart geometry ownership remains separate',records=records,total_frames=len(records),shadow_key_pixels=sum(r['shadow_key_pixels']for r in records),semantics={'transparent':'Source green key already decoded to alpha0; source RGB preserved.','shadow':'Exact RGB[0,0,255] is the serialized shadow key0x001F, applied by the shadow renderer, not blue object paint. Rendering mode remains bound per instance.','mobile':'Mobile cart-named profiles depict the detached horse team and harness; do not promote these pixels to cart scenery geometry.','target':'Target sources contain both cart and horse pixels; this body mask does not claim they are one static object.'},limitations=['Dynamic shadow domains require moving target placement and exact native blend mode; these are not static ground masks.','No animal silhouettes, cart panels or wheel geometry inferred automatically.','No source frames modified and no visibility/background patch records replaced.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print({'frames':len(records),'shadow_key_samples':result['shadow_key_pixels']})
if __name__=='__main__':main()
