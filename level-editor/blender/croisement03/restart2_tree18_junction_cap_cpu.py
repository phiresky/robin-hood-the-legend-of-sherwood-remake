"""Remove an unsupported broad continuation cap from the branching junction."""
import copy
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
import restart2_tree18_root_connectivity as topology
import restart2_tree18_cpu_wood as sweeps
B=Path(__file__).resolve().parents[3]/'level-editor/work/croisement03-refinement/restart2'


def main():
    source=B/'tree18-five-track-cpu-v2/geometry.json';out=B/'tree18-five-track-cpu-v3';assert not out.exists();original=json.loads(source.read_text());revised=copy.deepcopy(original);part=next(p for p in revised['parts']if p['track']=='main_lower');old=next(p for p in original['parts']if p['track']=='main_lower');n=part['ring_sides'];last=len(part['ring_roles'])-1;assert part['ring_roles'][-1]=='inferred_hidden_continuation'
    vertices=np.array(part['vertices']);previous=vertices[(last-1)*n:last*n].mean(0);oldcenter=vertices[last*n:(last+1)*n].mean(0);direction=oldcenter-previous;length=np.linalg.norm(direction);direction/=length;newcenter=previous+direction*3.;delta=newcenter-oldcenter;vertices[last*n:(last+1)*n]+=delta;vertices[-1]+=delta;part['vertices']=vertices.tolist()
    for guard in part['source_endpoint_guards']:assert part['vertices'][guard['vertex']]==old['vertices'][guard['vertex']]
    crossings=topology.crossing_pairs(part);assert not crossings;triangles=vertices[np.array(part['faces'])];volume=float(np.einsum('ij,ij->i',triangles[:,0],np.cross(triangles[:,1],triangles[:,2])).sum()/6);assert volume>0;part['volume']=volume;part['bounds_world']=[vertices.min(0).tolist(),vertices.max(0).tolist()]
    revised['status']='PRIVATE_SOURCE_SUPPORTED_JUNCTION_CORRECTION_SAVED_UNION_PENDING';revised['junction_cap_revision']={'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'old_extension_world':float(length),'new_extension_world':3.,'old_cap_projected_center':sweeps.project(oldcenter).tolist(),'new_cap_projected_center':sweeps.project(newcenter).tolist(),'old_world_center':oldcenter.tolist(),'new_world_center':newcenter.tolist(),'old44_source_endpoint_vertices_unchanged':True,'self_crossings':crossings,'rationale':'Lower-trunk sweep automatically extended two radii beyond its last source section. Its broad flat cap occupied the gap between separate ascending stems. Shortening only inferred continuation leaves the four upper sweeps to form the source-supported fork.','limits':['Retain original cap geometry and provenance in prior CPUv2. This is source-anatomy correction, not adjusting depths to pass a native-plane metric.','Saved five-part Boolean union and first hits still required. Known bark/stone/source pixels remain protected.']}
    out.mkdir();(out/'geometry.json').write_text(json.dumps(revised,indent=2)+'\n');box=(385,340,470,425);src=Image.open(B.parent/'baseline/covered.png').convert('RGB').crop(box).resize((510,510),Image.Resampling.NEAREST);sheet=Image.new('RGB',(1530,545),'#222222');d=ImageDraw.Draw(sheet)
    for i,(label,p)in enumerate([('Unmarked source',None),('Old inferred cap',old),('Shortened inferred cap',part)]):
        sheet.paste(src,(i*510,30));d.text((i*510+5,5),label,fill='white')
        if p:
            vv=np.array([sweeps.project(v)for v in p['vertices']]);ids=list(range(last*n,(last+1)*n));points=[tuple((vv[j]-box[:2])*6+np.array([i*510,30]))for j in ids];d.line(points+[points[0]],fill='cyan',width=2)
    sheet.save(out/'junction-cap-source-comparison.png');print(revised['junction_cap_revision'])


if __name__=='__main__':main()
