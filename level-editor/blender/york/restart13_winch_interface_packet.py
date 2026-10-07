"""Draw measured source overlays and physical sections without new 3D renders."""
import hashlib, json, math
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement'
OUT = WORK / 'restart2/winch-shaft-return-candidate-v1/interface-sections-v1'
data = json.loads((OUT / 'geometry.json').read_text())
s, c = data['projection']['sine'], data['projection']['cosine']
font = ImageFont.truetype('/usr/share/fonts/noto/NotoSans-Regular.ttf', 15)
small = ImageFont.truetype('/usr/share/fonts/noto/NotoSans-Regular.ttf', 12)
colors = {'Angled left frame brace.001': '#64c8ff', 'Frame top saddle.001': '#ffcf50'}
objects = {o['name']: o for o in data['objects']}

def hull(points):
    points = sorted(set(points))
    def cross(a, b, p):
        return (b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0])
    lower, upper = [], []
    for p in points:
        while len(lower) > 1 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(points):
        while len(upper) > 1 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1]+upper[:-1]

source = WORK / 'geometry-pass-01/native-state-source-v1'
record = next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004')
frames = next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')
canvas = Image.new('RGB', (1320, 690), '#20252d')
draw = ImageDraw.Draw(canvas)
draw.text((18, 12), 'Native source and frozen support outlines; red = exact chain/body intersections in final hypothesis', font=font, fill='white')
draw.text((18, 38), 'Blue: front left brace. Yellow: front saddle. Source art is unchanged; overlays are model projections, not source ownership.', font=small, fill='#cccccc')
for col, frame in enumerate((0,22,44)):
    f = frames[frame]
    tile = Image.new('RGBA', (50,104), '#303840')
    tile.alpha_composite(Image.open(source/f['image']).convert('RGBA'),(f['bbox'][0]-2385,f['bbox'][1]-875))
    clean=tile.resize((200,416),Image.Resampling.NEAREST).convert('RGB')
    x=col*440+10
    canvas.paste(clean,(x,95)); canvas.paste(clean,(x+220,95))
    draw.text((x,68),f'Frame {frame}: source / support overlay',font=font,fill='white')
    project=lambda p:(x+220+(p[0]-2385)*4,95+(-p[1]*s-p[2]*c-875)*4)
    for name,color in colors.items():
        polygon=hull([project(p) for p in objects[name]['vertices']])
        draw.line(polygon+[polygon[0]],fill=color,width=2)
    if frame==44:
        for row in data['crossings']:
            for p in row['points']:
                px,py=project(p);draw.ellipse((px-1,py-1,px+1,py+1),fill='#ff5b67')
    for y in (930,940,950,960,970):
        yy=95+(y-875)*4
        draw.line((x+215,yy,x+220,yy),fill='white')
        draw.text((x+174,yy-6),str(y),font=small,fill='white')
draw.text((18,535),'Measured crossing region: left brace screen Y 950.18–963.48; saddle Y 949.09–952.40.',font=font,fill='white')
draw.text((18,566),'The upper straight strand hits the saddle before the inferred lower wrap. Changing only the hidden return cannot fix that entry.',font=small,fill='white')
draw.text((18,594),'Source pixels at this scale do not determine front/back depth at the wood/chain overlap. A source-preserving depth revision remains plausible.',font=small,fill='white')
draw.text((18,622),'Neither the frozen support depth nor the helical return is proven by the source. The failed loop is withheld from approval.',font=small,fill='#ffb4bb')
canvas.save(OUT/'annotated-source.png')

canvas=Image.new('RGB',(1220,690),'#20252d');draw=ImageDraw.Draw(canvas)
draw.text((18,12),'Physical sections through the left chain entry: game Y / game Z',font=font,fill='white')
draw.text((18,38),'Blue front brace; yellow saddle; green drum/shaft; gray other body; red chain. These are plane cuts, not perspective views.',font=small,fill='white')
for index,xcut in enumerate((2399.5,2400.5)):
    left=65+index*600;top=100;w=500;h=485
    project=lambda p:(left+(-p[1]*s-1046)/30*w,top+(122-p[2]*c)/36*h)
    draw.text((left,70),f'Exact plane native X = {xcut}',font=font,fill='white')
    for y in range(1050,1077,5):
        xx=left+(y-1046)/30*w;draw.line((xx,top,xx,top+h),fill='#39414b');draw.text((xx-14,top+h+8),str(y),font=small,fill='white')
    for z in range(90,123,5):
        yy=top+(122-z)/36*h;draw.line((left,yy,left+w,yy),fill='#39414b');draw.text((left-29,yy-6),str(z),font=small,fill='white')
    for o in data['objects']:
        color=colors.get(o['name'],'#79bf99' if 'drum' in o['name'].lower() or 'axle' in o['name'].lower() else '#69717c')
        if o['name'].startswith('Shaft return'):color='#ff626f'
        vv=o['vertices']
        for face in o['faces']:
            hits=[]
            for ai,bi in zip(face,face[1:]+face[:1]):
                a,b=vv[ai],vv[bi]
                if (a[0]-xcut)*(b[0]-xcut)<0:
                    t=(xcut-a[0])/(b[0]-a[0]);hits.append([a[j]+t*(b[j]-a[j]) for j in range(3)])
            if len(hits)==2:
                pts=[project(p) for p in hits]
                if all(left-2<=px<=left+w+2 and top-2<=py<=top+h+2 for px,py in pts):draw.line(pts,fill=color,width=2)
    draw.text((left+190,top+h+32),'game Y',font=font,fill='white')
draw.text((18,651),'Diagnostic only. No wood changed, no topology accepted, and no complete-mechanism approval requested.',font=small,fill='#ffb4bb')
canvas.save(OUT/'physical-sections.png')
receipt={'status':'Private HOLD: source/depth ambiguity at chain entry','source_frames':[0,22,44],'model_sha256':data['model_sha256'],'exact_crossing_pair_counts':data['body_crossing_triangle_pair_counts'],'images':[{ 'path':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in (OUT/'annotated-source.png',OUT/'physical-sections.png')],'geometry':{'path':'geometry.json','sha256':hashlib.sha256((OUT/'geometry.json').read_bytes()).hexdigest()},'decision_needed':'Choose physically supported entry inference before any support replacement; do not approve the intersecting loop.'}
(OUT/'packet.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps(receipt,indent=2))
