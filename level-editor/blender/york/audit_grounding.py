"""Audit the complete retained York scene against the terrain and prior geometry."""
import json
import argparse
import math
import sys
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'
sys.path.insert(0,str(Path(__file__).parent))
from verify_grounding import terrain_height
from floor_contacts import load_contacts, height_at


def height_inside(x,y,tri):
    # Ignore points within 0.003 scene units of a footprint edge. Float32
    # storage cannot reliably distinguish which side of that seam they occupy.
    for a,b,c in [(tri[0],tri[1],tri[2]),(tri[1],tri[2],tri[0]),(tri[2],tri[0],tri[1])]:
        length=math.hypot(b[0]-a[0],b[1]-a[1])
        if length<1e-8:return None
        distance=((b[0]-a[0])*(y-a[1])-(b[1]-a[1])*(x-a[0]))/length
        side=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
        if distance*(1 if side>=0 else -1)<.003:return None
    return terrain_height(x,y,tri)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--before',type=Path,default=OUT/'review-v4/geometry.json')
    args=parser.parse_args()
    before=json.loads(args.before.read_text())
    after=json.loads((OUT/'review/geometry.json').read_text())
    report=json.loads((OUT/'grounding/report.json').read_text())
    changed={(r['source_node'],r['component']) for r in report['changes']}
    def parts(doc):return {(p['source_node'],p['component']):p for g in doc.values() for p in g['parts']}
    old,new=parts(before),parts(after)
    assert old.keys()==new.keys()
    drift=0
    for key,part in new.items():
        if key in changed:continue
        previous=old[key]
        assert previous['triangles']==part['triangles']
        assert len(previous['positions'])==len(part['positions'])
        drift=max(drift,max((abs(a-b) for p,q in zip(previous['positions'],part['positions']) for a,b in zip(p,q)),default=0))
    assert drift<.003,drift
    supports=[]
    for key,part in new.items():
        if key[0] not in report['support_sources']:continue
        for tri in part['triangles']:
            pts=[part['positions'][i] for i in tri]
            lo=[min(p[k] for p in pts) for k in range(3)]
            hi=[max(p[k] for p in pts) for k in range(3)]
            supports.append((pts,lo,hi))
    suspects=[];samples=0
    for key,part in new.items():
        if key[0]=='ground' or key[0] in report['support_sources']:continue
        count=0;depth=0
        for tri in part['triangles']:
            pts=[part['positions'][i] for i in tri]
            for weights in [(1/3,1/3,1/3),(.8,.1,.1),(.1,.8,.1),(.1,.1,.8)]:
                p=[sum(v[k]*w for v,w in zip(pts,weights)) for k in range(3)];samples+=1
                for vertices,lo,hi in supports:
                    if p[2]>=hi[2]-.003 or p[2]<-.1 or any(p[k]<lo[k] or p[k]>hi[k] for k in [0,1]):continue
                    z=height_inside(p[0],p[1],vertices)
                    if z is not None and p[2]<z-.003:
                        count+=1;depth=max(depth,z-p[2]);break
        if count:suspects.append({'source':key,'samples':count,'max_depth':depth})
    contacts = load_contacts()
    floor_checks = []
    for identity, contact in contacts.items():
        points = [p for part in after[identity]['parts'] for p in part['positions']]
        low = min(p[2] for p in points)
        depth = max(height_at(contact,p[0],p[1])-p[2] for p in points)
        floor_checks.append({'asset':identity,'minimum_z':low,'maximum_depth_below_floor':depth})
        if depth > .003:
            suspects.append({'asset':identity,'max_depth':depth,'reason':'Below reviewed floor continuation'})
    result={'status':'PASS' if not suspects else 'FAIL','unchanged_components':len(old)-len(changed),
        'floor_continuations':floor_checks,
        'max_unchanged_position_drift':drift,'retained_surface_samples_checked':samples,'buried_candidates':suspects}
    (OUT/'grounding/coverage-audit.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))
    if suspects:raise ValueError('Buried surfaces remain')


if __name__=='__main__':main()
