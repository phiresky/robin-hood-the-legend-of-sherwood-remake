"""Run the pinned bounded builder with the corrected independent source proposal.

The original candidate and its recipe stay immutable. This small adapter only
replaces the proposal, destination, variant fit and source-ownership gate.
"""
import hashlib
from pathlib import Path
BASE=Path(__file__).with_name('restart20_timber_candidate.py')
text=BASE.read_text()
assert hashlib.sha256(BASE.read_bytes()).hexdigest()=='585f93ec52b1fa1334e9bb5f6f7b6654729bd1360c8d937a780c47707fea94df'
def replace(old,new):
 global text
 assert text.count(old)==1,old
 text=text.replace(old,new)
replace("OUT=WORK/'restart2/loose-planks-candidate-v1'","OUT=WORK/'restart2/loose-planks-candidate-v2'")
replace("PLAN=WORK/'restart2/timber-contact-plan-v1/plan.json'","PLAN=WORK/'restart2/timber-corrected-plan-v2/plan.json'")
replace('ec34bcd8e3b98fa5a7e5a957e0b55b86684f7f4ae8096ebfe5c720eddad9bf12','da9ac4ed812662143486c200dccb17bb51a2716bc9b724b4029a460b9e5dadde')
start=text.index('# Rectangular rightmost variant,');end=text.index('bpy.ops.wm.read_factory_settings',start)
text=text[:start]+"best={'variant':'Independent top/end trace; see CPU proposal targets','maximum_error':None}\n"+text[end:]
injection='''# Per-piece and role cores stay independent of this geometry's first-hit result.
domain_root=WORK/'restart2/loose-planks-ownership-v3'
source_owners={}
for domain in sorted((domain_root/'domains').glob('*.png')):
    piece,role=domain.stem.rsplit('-',1);im=Image.open(domain).convert('L')
    for yy in range(im.height):
        for xx in range(im.width):
            if im.getpixel((xx,yy)):source_owners[(xx+1240,yy+920)]=(piece,role)
def inside_polygon(x,y,poly):
    inside=False
    for a,b in zip(poly,poly[1:]+poly[:1]):
        if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:inside=not inside
    return inside
for xy,owner in list(source_owners.items()):
    if owner[0].startswith('pale') and inside_polygon(xy[0]+.5,xy[1]+.5,plan['source_long_side_polygon']):
        source_owners[xy]=('long-crossing','side')
role_rejected=0
'''
replace('accepted=0;outside_mask=0;mask_uncovered=0',injection+'accepted=0;outside_mask=0;mask_uncovered=0')
replace("        images[owners[index]].putpixel((x-box[0],y-box[1]),art.getpixel((x,y)));accepted+=1",'''        object_name,face_index=owners[index]
        obj=scene.objects[object_name];face_normal=obj.data.polygons[face_index].normal
        face_role='top' if face_normal.z>.99 else 'bottom' if face_normal.z<-.99 else 'side'
        expected=(obj.data.name,face_role)
        if source_owners.get((x,y))!=expected:role_rejected+=1;continue
        images[owners[index]].putpixel((x-box[0],y-box[1]),art.getpixel((x,y)));accepted+=1''')
replace("'accepted_pixel_centers':accepted,","'accepted_pixel_centers':accepted,'outside_independent_role_core':role_rejected,'independent_core_pixels':len(source_owners),")
replace("Mask005 includes foreign ground; rejected mask pixels are not automatically missing wood. Per-face first-hit source diagnostic requires visual ownership review.","Independent per-piece top/side cores gate every accepted source pixel; uncertain boundaries remain gray. Manual core trace and shape still need visual review.")
exec(compile(text,str(BASE),'exec'),{'__file__':str(BASE),'__name__':'__main__'})
