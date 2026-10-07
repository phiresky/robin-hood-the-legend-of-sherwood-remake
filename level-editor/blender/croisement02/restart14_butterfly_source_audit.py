"""Freeze butterfly source domains, phase timing and observable silhouettes."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import label
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/croisement02-refinement'
source=WORK/'restart10-ambient-source-audit-v1/report.json';out=WORK/'restart14-butterflies/source-audit-v1';out.mkdir(parents=True,exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();rows=[]
for a in json.loads(source.read_text())['animations']:
 if 'papillon' not in a['profile']:continue
 sheet=Image.new('RGB',(1100,900),'#252525');draw=ImageDraw.Draw(sheet);records=[];tick=0
 for i,f in enumerate(a['frames']):
  p=Path(f['file']);assert sha(p)==f['sha256'];im=Image.open(p).convert('RGBA');arr=np.array(im);alpha=arr[:,:,3]>0
  labs,n=label(alpha,np.ones((3,3)));sizes=sorted([int((labs==k).sum()) for k in range(1,n+1)],reverse=True)
  records.append({'index':i,'first_tick':tick,'duration_ticks':f['duration_ticks'],'source':str(p),'sha256':sha(p),'bbox':f['bbox'],'opaque_pixels':int(alpha.sum()),'component_sizes':sizes,'alpha_centroid_display':f['alpha_centroid_display']});tick+=f['duration_ticks']
  scale=max(1,min(90//im.width,70//im.height));zoom=im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST);x=i%11*100;y=i//11*100;sheet.paste(zoom,(x+5,y+20),zoom);draw.text((x+5,y+4),str(i),fill='white')
 name=f"animation-{a['index']:02d}-all99.png";sheet.save(out/name)
 rows.append({'index':a['index'],'profile':a['profile'],'sprite':a['sprite'],'frame_count':len(records),'cycle_ticks':tick,'temporal_bbox':a['temporal_bbox'],'sheet':name,'frames':records,'empty_frames':[r['index'] for r in records if r['opaque_pixels']==0],'count_interpretation':'Disconnected antialiased wing/body islands are not automatically separate butterflies; contact-sheet review required.'})
report={'source_sha256':sha(source),'sequences':rows,'prototype_index':8,'physical_depth':'Unobserved; source elevation is display ordering, not flight height. Projected anchors constrain x/y only.','timing':'Each serialized delay=1 advances after two ticks;99frames198ticks. No added interpolated source poses.','butterfly05_semantics':'Elevation-zero FX uses captured background restore and special1x1 empty-frame handling; source99frames presently all nonempty. Preserve this distinct renderer behavior even though physical prototype is separate.'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['index'],r['frame_count'],r['cycle_ticks'],r['empty_frames']) for r in rows])
