"""Draw the recorded source-corner trace without altering its source image."""
import json, math
import shutil
from pathlib import Path
from PIL import Image, ImageDraw
ROOT=Path(__file__).resolve().parents[2]
N=ROOT/'level-editor/work/needs-review/derby-refinement/round-2/assets/derby-great-keep/component-followups/north'
OUT=N/'pixel-trace-v4'
data=json.loads((OUT/'trace.json').read_text())
source=Image.open(N/'approval-final/context.png').convert('RGB')
(OUT/'before').mkdir(exist_ok=True)
for name in ['solid.png','textured.png','context.png']:
    shutil.copyfile(N/'approval-final'/name,OUT/'before'/name)
for label,points in data['source_trace'].items():
    bounds=(min(p[0] for p in points)-10,min(p[1] for p in points)-10,max(p[0] for p in points)+10,max(p[1] for p in points)+10)
    box=tuple(int(p) for p in bounds);scale=5
    raw=source.crop((box[0]-834,box[1]-27,box[2]-834,box[3]-27)).resize(((box[2]-box[0])*scale,(box[3]-box[1])*scale),Image.Resampling.NEAREST)
    overlay=raw.copy();d=ImageDraw.Draw(overlay)
    pts=[((p[0]-box[0])*scale,(p[1]-box[1])*scale) for p in points]
    d.line(pts,fill='cyan',width=1)
    for i,(x,y) in enumerate(pts):
        d.ellipse((x-2,y-2,x+2,y+2),fill='yellow')
        tx=x+5;ty=y+(-16 if i%2==0 else 5)
        d.text((tx,ty),str(i+1),fill='yellow',stroke_width=1,stroke_fill='black')
    raw.save(OUT/f'{label}-source.png');overlay.save(OUT/f'{label}-trace.png')
report=['# North Tower — source-corner trace v4','','V3 is withdrawn: it confused hidden baseline coordinates with the visible Working geometry. This packet works in verified world coordinates.','',
        'The yellow dots below are individual selected source pixels, numbered in path order. Cyan segments join the cap/notch arris. Original artwork is shown beside every annotation. Selection uncertainty is about 2 source pixels; the construction residual is not independent evidence that a selected pixel is correct.','']
for label,points in data['source_trace'].items():
    report += [f'## {label.title()} curtain','',f'![Unmodified source crop]({label}-source.png)','',f'![Numbered corner trace]({label}-trace.png)','', '| Point | Source X | Source Y |','|---:|---:|---:|']
    report += [f'| {i+1} | {x} | {y} |' for i,(x,y) in enumerate(points)]
    report += ['']
audit=json.loads((OUT/'mesh-audit.json').read_text())
for label,points in data['source_trace'].items():
    box=(int(min(p[0] for p in points))-10,int(min(p[1] for p in points))-10,int(max(p[0] for p in points))+10,int(max(p[1] for p in points))+10)
    im=source.crop((box[0]-834,box[1]-27,box[2]-834,box[3]-27)).resize(((box[2]-box[0])*5,(box[3]-box[1])*5),Image.Resampling.NEAREST);d=ImageDraw.Draw(im)
    for p in points:
        row=min(audit['samples'],key=lambda row:math.dist(row['expected'],p))
        a=((p[0]-box[0])*5,(p[1]-box[1])*5);b=((row['actual'][0]-box[0])*5,(row['actual'][1]-box[1])*5)
        d.ellipse((a[0]-4,a[1]-4,a[0]+4,a[1]+4),outline='yellow',width=1)
        d.line((b[0]-3,b[1],b[0]+3,b[1]),fill='cyan');d.line((b[0],b[1]-3,b[0],b[1]+3),fill='cyan')
        d.line((a,b),fill='red')
    im.save(OUT/f'{label}-residual.png')
    report += [f'![{label} saved-mesh fit: yellow observed, cyan projected]({label}-residual.png)','']
initial=Image.open(N.parent.parent/'reference/mission-patches/H03_Der_MK-initial.png').convert('RGB')
revealed=Image.open(N.parent.parent/'reference/revealed.png').convert('RGB')
box=(900,150,1170,335)
a=initial.crop(box);b=revealed.crop(box)
changed=sum(p!=q for p,q in zip(a.getdata(),b.getdata()))
a.resize((810,555),Image.Resampling.NEAREST).save(OUT/'covered-source.png')
b.resize((810,555),Image.Resampling.NEAREST).save(OUT/'revealed-source.png')
report += ['## Covered and revealed state','','![Covered original top of tower](covered-source.png)','','![Revealed original top of tower](revealed-source.png)','',f'The native covered/revealed artwork differs at {changed} pixels within this roof/curtain crop. North Tower nodes are not interior receivers for Keep patches 000/001; the source remains the covered exterior layer. Interior assignment must not be inferred from the whole-map revealed image.','',
           '## Saved-mesh contour residuals','',f'Maximum selected-corner residual: {audit["max_saved_mesh_corner_error"]:.5f} source pixels. Mean: {audit["mean_saved_mesh_corner_error"]:.5f}. Yellow circles are selected artwork points; cyan crosses are the saved mesh projected back into the source. This is a construction check, not independent annotation accuracy.','']
report += ['## Geometry and projection','','Before (historical lighting):','','![Before solid](before/solid.png)','','![Before source projection](before/textured.png)','','After (48° elevation):','','![Eight-view solid](modified-v2/solid.png)','','![Eight-view source projection](modified-v2/textured.png)','','Only original source pixels pass the existing reviewed native masks; source mapping is reapplied after geometry. Lighting is explicitly 48° elevation.','',
           'The original lower tower surfaces below z=835 are clipped without moving the retained vertices. Four Working meshes change: door, main parapet, rear parapet, upper outlook. No catalog or production model is updated.','',
           'The rear curtain has three merlons and two crenels. The main front trace has six explicit descending crenel transitions; the rear-left trace has three. Hidden attachments are inferred and not scored as observed source features.']
report += ['','## Remaining topology limitations','',audit['topology_limit'],'',
           '| Node | Faces | Boundary edges | Nonmanifold edges | Zero-area faces |','|---|---:|---:|---:|---:|']
report += [f'| {node} | {r["faces"]} | {r["boundary_edges"]} | {r["nonmanifold_edges"]} | {r["zero_area_faces"]} |' for node,r in audit['topology'].items()]
report += ['','## Retained-body closure audit','',
           'The original main wall already had 88 attachment/base boundary edges. The first traced draft introduced additional clipping boundaries (189 boundary / 192 nonmanifold in total). These are now capped at the z=835 clipping plane and the inherited z=461.86 base. The cap areas are 5605 and 5595 square units: the wall-body cross-sections, not a slab across the courtyard.', '',
           'The original outlook had 3631 boundary edges before welding duplicated vertices, or 97 after welding. The first rounded draft had 76 boundaries: its new circular underside plus three narrow 0.011-unit clipping slivers. The underside is capped at z=835, and only those slivers are collapsed. Existing corbels remain. The repaired main wall and outlook both have zero boundary and zero nonmanifold edges.', '',
           'Closeups below compare the pre-closure and closed body from the identical camera. Caps face into the body/below attachments, so visible silhouettes should remain unchanged.','']
for i in [0,2,4,6]:
    old=Image.open(OUT/f'modified-v2/views/view-{i}-solid.png').convert('RGB')
    new=Image.open(OUT/f'modified-v3/views/view-{i}-solid.png').convert('RGB')
    # Upper half includes the retained-body seam, outlook and crenellations.
    box=(0,0,old.width,int(old.height*.65))
    old=old.crop(box);new=new.crop(box)
    pair=Image.new('RGB',(old.width*2,old.height+24));pair.paste(old,(0,24));pair.paste(new,(old.width,24))
    d=ImageDraw.Draw(pair);d.text((10,5),'Before closure',fill='white');d.text((old.width+10,5),'Closed retained bodies',fill='white')
    pair.save(OUT/f'closure-view-{i}.png')
    report += [f'![Closure comparison, view {i}](closure-view-{i}.png)','']
report += ['This remains an isolated geometry review candidate. Adjacent closed solids are not Boolean-unioned; hidden wall returns remain inferred.']
(OUT/'review.md').write_text('\n'.join(report).replace('modified-v2/solid.png','modified-v3/solid.png').replace('modified-v2/textured.png','modified-v3/textured.png')+'\n')
