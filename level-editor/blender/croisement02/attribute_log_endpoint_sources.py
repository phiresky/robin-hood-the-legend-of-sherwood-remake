"""Attribute surveyed endpoint wood to native timed parts without inventing motion."""
import hashlib,json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.spatial import ConvexHull
from catalog import OUT
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def main():
    root=OUT/'state-target-evidence/log-trap';manifest=json.loads((root/'manifest.json').read_text());fit=json.loads((root/'applied-cylinder-fit.json').read_text());box=manifest['bbox'];w=box[2]-box[0];h=box[3]-box[1];owners=np.full((h,w),-1,dtype=np.int16);parts=[]
    for index,part in enumerate(manifest['parts']):
        frame=part['frames'][-1];x,y=[int(math.floor(a+b+.5))for a,b in zip(part['position'],frame['offset'])];a=np.array(Image.open(frame['image']))[:,:,3]>0;region=owners[y-box[1]:y-box[1]+a.shape[0],x-box[0]:x-box[0]+a.shape[1]];region[a]=index;parts.append(dict(profile=part['profile'],terminal_frame=frame['index'],terminal_reached_tick=part['terminal_frame_reached_tick'],source_sha256=frame['image_sha256']))
    angles=np.arange(16)*math.tau/16;records=[]
    for i,(ax,ay,bx,by,r,z)in enumerate(fit['survey']):
        tangent=np.array([bx-ax,-(by-ay)/SIN]);tangent/=np.linalg.norm(tangent);cross=np.array([-tangent[1],-tangent[0]*SIN]);ring=r*(np.cos(angles)[:,None]*cross+np.sin(angles)[:,None]*[0,-COS]);points=np.concatenate([ring+[ax,ay],ring+[bx,by]]);hull=ConvexHull(points);canvas=Image.new('L',(w,h));ImageDraw.Draw(canvas).polygon([tuple(v)for v in points[hull.vertices]],fill=255);domain=np.array(canvas)>0;counts=[int((domain&(owners==j)).sum())for j in range(len(parts))];total=sum(counts);best=max(range(len(counts)),key=counts.__getitem__)
        records.append(dict(survey_index=i,visible_pixels_by_native_part=counts,dominant_native_part=parts[best]['profile']if total else None,dominance=counts[best]/total if total else 0,status='endpoint appearance attribution only; no across-frame identity'))
    visibility=[]
    for tick in range(manifest['terminal_geometry_reached_tick']+2):
        active=[]
        for part in manifest['parts']:
            clock=part['start_tick'];selected=part['initial']
            if tick>=clock:
                for selected in part['frames']:
                    clock+=selected['ticks']
                    if tick<clock:break
            if selected['opaque_pixels']:active.append(part['profile'])
        if visibility and visibility[-1]['active_profiles']==active:visibility[-1]['last_tick']=tick
        else:visibility.append(dict(first_tick=tick,last_tick=tick,active_profiles=active))
    assert all(len(r['active_profiles'])==1 for r in visibility),visibility
    report=dict(visible_stage_runs=visibility,visibility_semantics='All three native target actions start together; transparent lead/terminal frames make successive whole-assembly stages. Relative sprite ticks do not assert script-scheduler phase.',status='Evidence, not an animation rig',manifest_sha256=hashlib.sha256((root/'manifest.json').read_bytes()).hexdigest(),fit_sha256=hashlib.sha256((root/'applied-cylinder-fit.json').read_bytes()).hexdigest(),native_parts=parts,survey_attribution=records,limitations=['Multiple surveys may overlap and sample the same native pixel; this report does not assign exclusive geometry ownership.','Dominant endpoint appearance cannot establish a rigid log correspondence through prior frames.','Timing remains bound to the three native target profiles independently.'])
    (root/'applied-part-attribution.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['survey_index'],r['dominant_native_part'],round(r['dominance'],3))for r in records])
if __name__=='__main__':main()
