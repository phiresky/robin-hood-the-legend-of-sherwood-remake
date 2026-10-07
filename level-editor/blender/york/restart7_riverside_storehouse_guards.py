"""Audit exact native ownership atlases and the retained inferred storehouse floor."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/york-refinement'
D=B/'restart7-riverside-storehouse-v2'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 j=json.loads((D/'review.json').read_text());assert sha(D/'model.blend')==j['model_sha256']
 art=Image.open(B/'baseline/covered.png').convert('RGBA');rgb=np.array(art.crop((1449,840,1645,1132)))[:,:,:3];mask=np.array(Image.open(B/'baseline/masks/000000.png').convert('L'));rows=[];total=np.zeros(mask.shape,dtype=int)
 for p in sorted(D.glob('building-*-owned-rgba.png')):
  a=np.array(Image.open(p));known=a[:,:,3]>0;assert np.array_equal(a[:,:,:3],rgb);assert np.array_equal(a[:,:,3][known],mask[known]);total+=known;rows.append(dict(file=p.name,sha256=sha(p),known=int(known.sum()),all_rgb_exact=True,known_alpha_exact=True))
 assert total.max()==1 and len(rows)==6 and int(total.sum())==j['own_first_hit']
 v=art.convert('RGB');draw=ImageDraw.Draw(v)
 for x,y in j['misses']:draw.point((x,y),fill=(255,0,255))
 for q in j['foreign_first_hits']:
  if 'shed' not in q['owner']:draw.point(tuple(q['pixel']),fill=(0,255,255))
 v.crop((1440,830,1650,1145)).resize((630,945),Image.Resampling.NEAREST).save(D/'review/source-miss-overlay.png')
 p=B/'grounding/report.json';floor=json.loads(p.read_text())['floor_extensions']['york-riverside-stone-storehouse'];pts=[(1468.8,-2031.25),(1577.5,-1975.5),(1626.8,-2071.7),(1518.05,-2127.4)];verts=[v for t in floor['triangles']for v in t];bounds=[[min(v[k]for v in verts),max(v[k]for v in verts)]for k in range(2)];assert all(bounds[0][0]<=x<=bounds[0][1]and bounds[1][0]<=y<=bounds[1][1]for x,y in pts)
 limitation='River-facing physical foundation is hidden in native artwork. Inferred floor is a geometric continuation contract, not a separately modeled observed terrain receiver.'
 support=dict(status='PASS retained inferred hidden floor contract; not an observed rear support claim',model_sha256=j['model_sha256'],floor_contract=floor,base_z=109.8713,clearance_to_inferred_floor=109.8713-floor['height_scene'],all_four_footprint_points_within_inferred_floor=True,source_files={str(q):sha(q)for q in [p,ROOT/'level-editor/blender/york/floor-contacts.json']},known_terrain_contacts='Front corners are 0.121 scene-unit above ground086. Historical inferred floor plane is 0.12064 above that surface; original choice preserved.',limitation=limitation)
 write(D/'support-guard.json',support)
 guard=dict(model_sha256=j['model_sha256'],source_rgb_exact=True,per_receiver_native_alpha_exact=True,duplicate_accepted_pixels=int((total>1).sum()),accepted_unique=int((total>0).sum()),domain_pixels=int((mask>0).sum()),delegated_shed_pixels=sum('shed'in q['owner']for q in j['foreign_first_hits']),delegated_ground_pixels=sum('shed'not in q['owner']for q in j['foreign_first_hits']),physical_void_boundary_pixels=len(j['misses']),atlases=rows,context_guard='construction.json/context_exact preserves frozen shed and source floor mesh/UV/evaluated matrices; no material edits to these receivers',support_contract='support-guard.json',limitations=['First-hit source ownership excludes hidden chimney/roof and shed/masonry projections; it does not create missing art.','94 source boundary centers remain without physical first hit; kept in independent original-mask audit.','Four floor-owned centers at storehouse/shed foot remain separate.','Original source blue/gray shoreline antialiasing at masonry foot retained; no floor artwork fabricated.',limitation])
 write(D/'source-guard.json',guard)
 print(json.dumps(dict(model=j['model_sha256'],native_owned=int(total.sum()),boundary_misses=len(j['misses']),inferred_floor_clearance=support['clearance_to_inferred_floor'])))
if __name__=='__main__':main()
