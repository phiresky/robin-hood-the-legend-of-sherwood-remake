"""Correct an inherited thin-cap depth hypothesis while preserving source trace projections."""
import hashlib
import json
from pathlib import Path
from collections import Counter

import numpy as np
from restart2_bank_main_mass_cpu import B, S, C, RAY, distances
from restart2_bank_boulder_faces import project

O=B/'bank-cap-depth-cpu-v2'


def main():
    assert not O.exists()
    source=B/'bank-main-masses-cpu-v1/geometry.json'
    data=json.loads(source.read_text());mesh=data['52'];old=np.array(mesh['vertices']);v=old.copy();xy=project(v)
    upper=next(t for t in mesh['traces']if t['id']=='west-upper')
    lower=next(t for t in mesh['traces']if t['id']=='west-lower')
    def trace_distance(trace):
        result=np.full(len(v),np.inf)
        for a,b in zip(trace['vertices'],trace['vertices'][1:]):
            result=np.minimum(result,distances(xy,xy[a],xy[b]))
        return result
    du,dl=trace_distance(upper),trace_distance(lower)
    # A20-unit cap band replaces the old5-unit hypothesis. Native source
    # anchors determine projected creases; they do not measure world depths.
    authored_drop=15.
    weight=np.exp(-(dl/24.)**2)*np.minimum(1.,du/np.maximum(du+dl,1e-12))
    weight[dl<1e-8]=1
    weight[du<1e-8]=0
    weight[v[:,2]<1e-7]=0
    weight[mesh['boundary_top_indices']]=0
    # Two full subdivisions retained16 consecutive children per input face.
    # Freeze every vertex incident to a side/bottom face, not merely the old
    # corner columns; intermediate boundary points must remain fixed too.
    source0=json.loads((B/'bank-convex-masses-cpu-v1/geometry.json').read_text())['52']
    assert len(mesh['faces'])==16*len(source0['faces'])
    fixed={vertex for i,face in enumerate(mesh['faces'])
           if not all(v<source0['source_vertex_count']for v in source0['faces'][i//16])
           for vertex in face}
    weight[list(fixed)]=0
    v-=weight[:,None]*RAY*(authored_drop/C/S)
    assert np.max(np.abs(project(v)-xy))<1e-9
    assert np.max(np.abs(v[upper['vertices']]-old[upper['vertices']]))<1e-9
    assert np.max(np.abs(v[lower['vertices'],2]*C-65))<1e-8
    assert v[:,2].min()>=-1e-7
    assert np.array_equal(v[mesh['boundary_top_indices']],old[mesh['boundary_top_indices']])
    assert np.array_equal(v[list(fixed)],old[list(fixed)])
    counts=Counter(tuple(sorted((a,b)))for f in mesh['faces']for a,b in zip(f,f[1:]+f[:1]))
    assert set(counts.values())=={2}
    volume=sum(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]]))/6 for f in mesh['faces']);assert volume>0
    mesh['vertices']=v.tolist();mesh['signed_volume']=float(volume)
    mesh['cap_depth_revision']={'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
        'old_authored_band_heights':[85,80],'new_authored_band_heights':[85,65],
        'old_world':old.tolist(),'new_depths_are_inferred':True,
        'reason':'Inherited5-unit thickness produced a thin broad lip in saved oblique views. Source shows a substantial rounded upper stone; world depth is not fixed by source pixels.',
        'exact_source_2d_max_error':float(np.max(np.abs(project(v)-xy))),
        'fixed_side_bottom_vertices':len(fixed),
        'changed_vertices':int(np.any(np.abs(v-old)>1e-8,axis=1).sum()),
        'required':['Independent prior-source2D and fixedcrest proof','Saved source rays/contacts/topology','Full original-camera/oblique source and morphology selfreview']}
    O.mkdir();(O/'geometry.json').write_text(json.dumps(data,separators=(',',':'))+'\n')
    print(mesh['cap_depth_revision']['changed_vertices'])


if __name__=='__main__':main()
