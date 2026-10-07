"""Read-only construction audit of stored basal sweep profiles, without Blender."""
import argparse,hashlib,json,math
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=np.array([0.,-COS,SIN])

def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def audit_profile(rows,ground,cut,front_half_ground):
    rows=np.array(rows,float);y=rows[:,0];kernel=np.array([1,2,3,2,1],float)/9
    cx=np.convolve(np.pad(rows[:,1],2,mode='edge'),kernel,mode='valid');radius=np.convolve(np.pad(rows[:,2],2,mode='edge'),kernel,mode='valid')
    z=(ground-y)/COS;centers=np.column_stack([cx,np.full(len(y),-ground/SIN),z]);depth=centers@RAY
    floor_center=np.full(len(y),.25) if front_half_ground else .25+radius*SIN
    start=687 if front_half_ground else 715
    shift=np.where(y>=start,np.maximum(0.,(floor_center-z)/SIN),0.);depth+=shift
    a=np.arange(48)*math.tau/48
    source=np.zeros((len(y),48,3));source[:,:,0]=cx[:,None]+radius[:,None]*np.cos(a);source[:,:,1]=-y[:,None]*SIN;source[:,:,2]=-y[:,None]*COS
    raw_depth=depth[:,None]+radius[:,None]*np.sin(a);limit=(cut-.25-source[:,:,2])/SIN
    vertices=source+np.minimum(raw_depth,limit)[:,:,None]*RAY
    tangent=np.diff(vertices,axis=0);norm=np.linalg.norm(tangent,axis=2);safe=norm>1e-10
    unit=np.divide(tangent,norm[:,:,None],out=np.zeros_like(tangent),where=safe[:,:,None]);valid=safe[:-1]&safe[1:]
    turn=np.degrees(np.arccos(np.clip(np.sum(unit[:-1]*unit[1:],axis=2),-1,1)));turn=turn[valid]
    gap=np.diff(y);slope=np.diff(radius)/gap;slope_change=np.diff(slope)
    worst=np.argsort(np.abs(slope_change))[-6:][::-1]
    jumps=np.argsort(np.abs(np.diff(rows[:,1])))[-4:][::-1]
    return dict(rows=len(rows),raw_row_center_jumps=[dict(source_y=[float(y[i]),float(y[i+1])],delta_x=float(rows[i+1,1]-rows[i,1])) for i in jumps],missing_source_row_intervals=[[float(y[i]),float(y[i+1])] for i in np.where(gap>1.01)[0]],radius_range=[float(radius.min()),float(radius.max())],radius_slope_sign_changes=int(np.sum(slope[:-1]*slope[1:]<0)),worst_radius_slope_changes=[dict(source_y=float(y[i+1]),change=float(slope_change[i])) for i in worst],sweep_turn_degrees=dict(maximum=float(turn.max()),p95=float(np.quantile(turn,.95)),over15_count=int(np.sum(turn>15))),upper_cut_clamped_vertices=int(np.sum(raw_depth>limit)),upper_cut_total_vertices=int(raw_depth.size),ground_center_shift_range=[float(shift.min()),float(shift.max())],terminal_radius=float(radius[-1]),terminal_flat_cap_area=float(math.pi*radius[-1]**2),terminal_source_y=float(y[-1]))

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();result=[]
    source=json.loads((OUT/'forest-v4-sources/manifest.json').read_text())
    for index,relative,cut in [(32,'tree32-root-research/continuous-fork-v4',100),(38,'tree38-root-research/continuous-contour-v9',115)]:
        directory=OUT/relative;evidence=directory/'evidence.json';d=json.loads(evidence.read_text());profiles=d.get('source_profiles',d.get('profiles'));ground=next(r['ground_y'] for r in source if r['mask']==index)
        result.append(dict(tree=index,construction_evidence=str(evidence),construction_evidence_sha256=digest(evidence),model_sha256=d['model_sha256'],existing_topology_guard=d.get('full_geometry',d.get('geometry')),profiles={role:audit_profile(rows,ground,cut,index==38) for role,rows in profiles.items()},limitations=['Measurements reconstruct pre-union sweep from saved inputs, not final mesh vertices or normals.','Manifold/degenerate guards do not establish curvature continuity or source-supported depth.','Native artwork constrains projected outline and color; circular inferred depth is not independently observed.']))
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(dict(status='CPU construction diagnosis; no models or selections changed',records=result),indent=2)+'\n')
    for r in result:
        print(r['tree'],[(name,p['sweep_turn_degrees'],p['upper_cut_clamped_vertices'],p['terminal_radius']) for name,p in r['profiles'].items()])
if __name__=='__main__':main()
