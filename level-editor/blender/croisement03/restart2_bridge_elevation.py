"""Audit native bridge movement elevation before inferring river depth."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement03-refinement/baseline'
OUT=BASE.parent/'restart2/bridge-elevation'

def inside(point,vertices):
    x,y=point;hit=False
    for a,b in zip(vertices,vertices[1:]+vertices[:1]):
        if (a['y']>y)!=(b['y']>y) and x<(b['x']-a['x'])*(y-a['y'])/(b['y']-a['y'])+a['x']:hit=not hit
    return hit

def main():
    OUT.mkdir(parents=True,exist_ok=False);level_path=BASE/'Croisement03.rhp.json';level=json.loads(level_path.read_text());samples=[]
    for i in range(21):
        t=i/20;p=[880+(1024.5-880)*t,645.5+(743.5-645.5)*t]
        owners=[index for index,o in enumerate(level['sight_obstacles']) if inside(p,o['points'])];samples.append(dict(source_point=p,native_obstacles=owners))
    nearby=[r for r in level['elevation_lines'] if max(r['point_a'][0],r['point_b'][0])>=800 and min(r['point_a'][0],r['point_b'][0])<=1100 and max(r['point_a'][1],r['point_b'][1])>=590 and min(r['point_a'][1],r['point_b'][1])<=790]
    assert not nearby and not any(r['native_obstacles'] for r in samples)
    result=dict(level_sha256=hashlib.sha256(level_path.read_bytes()).hexdigest(),source_sha256=hashlib.sha256((BASE/'covered.png').read_bytes()).hexdigest(),bridge_center_samples=samples,nearby_elevation_lines=nearby,behavior_checked={'position_without_support_plane':'Map position becomes world position at elevation zero.','elevation_transition':'Crossing an elevation line switches the supporting obstacle plane.','source_inspection':'Native shoreline shows a visible bank face and pier below the deck; the global flat ground sheet cannot represent this river opening.'},decision='Set bridge deck top atZ0. Lower banks, water surface and riverbed remain explicitly inferred visual geometry.',water_depth='Unconstrained by native movement elevation. Private contact proof uses waterZ-35 and a bed contacting the pier; neither is an observed depth.',projection_ground_exclusion='The unrefined flat ground sheet crosses the visible below-land pier and river. Exclude it only from bridge source bake; actual joint land/water geometry is reviewed separately. Final scene ground replacement remains mandatory.')
    (OUT/'evidence.json').write_text(json.dumps(result,indent=2)+'\n');print('21 center samples: no supporting native obstacle; no nearby elevation transitions')

if __name__=='__main__':main()
