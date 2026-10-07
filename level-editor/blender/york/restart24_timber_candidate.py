"""Bounded model build using full independent observed faces and bevel fit."""
import hashlib
from pathlib import Path
BASE=Path(__file__).with_name('restart20_timber_candidate.py');text=BASE.read_text()
assert hashlib.sha256(BASE.read_bytes()).hexdigest()=='585f93ec52b1fa1334e9bb5f6f7b6654729bd1360c8d937a780c47707fea94df'
def replace(old,new):
 global text
 assert text.count(old)==1,old
 text=text.replace(old,new)
replace("OUT=WORK/'restart2/loose-planks-candidate-v1'","OUT=WORK/'restart2/loose-planks-candidate-v3'")
replace("PLAN=WORK/'restart2/timber-contact-plan-v1/plan.json'","PLAN=WORK/'restart2/timber-edge-fit-v1/plan.json'")
PLAN=BASE.resolve().parents[2]/'work/york-refinement/restart2/timber-edge-fit-v1/plan.json'
replace('ec34bcd8e3b98fa5a7e5a957e0b55b86684f7f4ae8096ebfe5c720eddad9bf12','82f33f4e67a2d7ae3038f8b602f0a06ab737b14ca2745df1883b57264c78d081')
start=text.index('# Rectangular rightmost variant,');end=text.index('bpy.ops.wm.read_factory_settings',start)
text=text[:start]+"best={'variant':'Full observed top and separately fitted lower edge','maximum_error':None}\n"+text[end:]
replace("verts=[(x,y,z) for z in (p['bottom_z'],p['top_z']) for x,y in p['footprint_world']]","verts=[(x,y,p['bottom_z']) for x,y in p['bottom_footprint_world']]+[(x,y,p['top_z']) for x,y in p['footprint_world']]")
replace('accepted=0;outside_mask=0;mask_uncovered=0',"""domain_path=WORK/'restart2/timber-observed-domains-v1/report.json'
assert hashlib.sha256(domain_path.read_bytes()).hexdigest()=='13b315ac0a225fd37d2ae4899291e10a488dafa573de61acfe15761eba8669c3'
domain_report=json.loads(domain_path.read_text())
source_owners={tuple(r['pixel']):tuple(r['owner']) for r in domain_report['pixels']}
role_rejected=0
accepted=0;outside_mask=0;mask_uncovered=0""")
replace('        if not masked: outside_mask+=1;continue','        if not masked: outside_mask+=1')
replace("        images[owners[index]].putpixel((x-box[0],y-box[1]),art.getpixel((x,y)));accepted+=1",'''        name,face_index=owners[index];obj=scene.objects[name];normal=obj.data.polygons[face_index].normal
        role='top' if normal.z>.99 else 'bottom' if normal.z<-.99 else 'side'
        if source_owners.get((x,y))!=(obj.data.name,role):role_rejected+=1;continue
        images[owners[index]].putpixel((x-box[0],y-box[1]),art.getpixel((x,y)));accepted+=1''')
replace("'accepted_pixel_centers':accepted,","'accepted_pixel_centers':accepted,'unmatched_observed_face_pixels':len(source_owners)-accepted,'independent_visible_pixels':len(source_owners),'other_model_pixels_unknown':role_rejected,")
replace('Mask005 includes foreign ground; rejected mask pixels are not automatically missing wood. Per-face first-hit source diagnostic requires visual ownership review.','Full per-piece observed top/side regions govern source; native occlusion mask is diagnostic only. Every unmatched observed face pixel remains a target; disputed shared-edge pixels stay separate.')
exec(compile(text,str(BASE),'exec'),{'__file__':str(BASE),'__name__':'__main__'})
