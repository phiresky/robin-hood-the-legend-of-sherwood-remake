"""Check native source-ray preservation before the inferred jamb-depth correction."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image
from shapely.geometry import Polygon
from shapely.ops import triangulate
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';D=W/'gate-saved-contact-audit-v1';OUT=W/'jamb-hidden-clearance-plan-v1';assert not OUT.exists();audit=json.loads((D/'report.json').read_text());proposal=json.loads((D/'hidden-clearance-proposal.json').read_text());old=np.array(audit['geometry_world']['jamb_vertices']);new=np.array(proposal['proposal']['proposed_jamb_vertices_world']);faces=audit['geometry_world']['jamb_faces'];normal=np.array(proposal['proposal']['normal_world']);axis=np.array([-normal[1],normal[0],0]);base=old[0]
def triangles(vs):
 out=[]
 for face in faces:
  if len(face)<=4:
   out.extend([vs[list((face[0],face[i],face[i+1]))]for i in range(1,len(face)-1)]);continue
  p=Polygon([(float((vs[i]-base)@axis),vs[i][2])for i in face]);depth=float((vs[face[0]]-base)@normal)
  for tri in triangulate(p):
   if not p.covers(tri.representative_point()):continue
   out.append(np.array([base+axis*u+normal*depth+np.array([0,0,z-base[2]])for u,z in list(tri.exterior.coords)[:3]]))
 return out
maskpath=W/'jamb-source-probe-v1/visible-jamb-domain.png';mask=np.array(Image.open(maskpath).convert('L'))>0;coords=np.argwhere(mask);pixels=[[int(x)+2250,int(y)+780]for y,x in coords];assert len(pixels)==660;s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);origins=np.array([[x+.5,-(y+.5)/s,0]for x,y in pixels])+back*10000;direction=-back
# Two-sided geometric rays; the authored domain has already excluded non-jamb owners.
def cast(tris):
 nearest=np.full(len(origins),np.inf)
 for a,b,d in tris:
  e1=b-a;e2=d-a;h=np.cross(direction,e2);det=e1@h
  if abs(det)<1e-10:continue
  q=origins-a;u=q@h/det;cross=np.cross(q,e1);v=cross@direction/det;t=cross@e2/det;hits=(u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)&(t>=0)&(t<nearest);nearest[hits]=t[hits]
 return nearest
before=cast(triangles(old));after=cast(triangles(new));both=np.isfinite(before)&np.isfinite(after);changed=[]
for i,p in enumerate(pixels):
 if not both[i]or abs(before[i]-after[i])>1e-4:changed.append({'pixel':p,'before_distance':float(before[i])if np.isfinite(before[i])else None,'after_distance':float(after[i])if np.isfinite(after[i])else None})
front_preserved=np.array_equal(old[13:],new[13:]);assert front_preserved
report={'status':'CPU_NATIVE_PROJECTION_GUARD','authoritative_domain':str(maskpath),'domain_sha256':hashlib.sha256(maskpath.read_bytes()).hexdigest(),'native_source_pixels':660,'front13vertices_exact':front_preserved,'unchanged_first_hit_pixels':660-len(changed),'changed_or_missing_rays':changed,'scope':'Ray positions only against extracted saved jamb geometry; source RGB/UV/material identity and retained context must also be verified in the later save recipe.','proposal':proposal['proposal'],'required_action':'Proceed only if every known source ray is unchanged, otherwise refine hidden-only correction boundaries.'};OUT.mkdir();(OUT/'projection-guard.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'source_pixels':660,'unchanged':660-len(changed),'changed':len(changed)}))
