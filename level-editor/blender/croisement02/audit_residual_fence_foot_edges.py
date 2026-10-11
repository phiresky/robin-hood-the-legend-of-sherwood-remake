"""Measure nearest installed fence contours without assigning texture ownership."""
import json
from collections import defaultdict
import numpy as np
import restart14_butterfly_canopy22_audit as core

OUT = core.W / 'static-terrain-closure-oct11-v1/fence-foot-edges-v1'
POINTS = {'croisement02-southwest-field-wattle-fence': [(571,1033),(571,1034),(571,1035),(572,1034),(572,1035),(573,1034)], 'croisement02-southeast-stone-wall-and-gate': [(1603,1046),(1604,1046),(1606,1047)]}
nearest = defaultdict(list)

def triangles(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
    asset = placed['id']
    st = np.stack((tri[:,:,0],core.SIN*tri[:,:,2]-core.COS*tri[:,:,1]),axis=2)
    for xy in POINTS[asset]:
        q=np.array(xy)+.5
        for edge in range(3):
            a=st[:,edge];v=st[:,(edge+1)%3]-a
            length=(v*v).sum(1);t=np.divide(((q-a)*v).sum(1),length,out=np.zeros(len(v)),where=length>1e-15).clip(0,1)
            p=a+t[:,None]*v;dist=((p-q)**2).sum(1);i=int(dist.argmin());world=tri[i,edge]+t[i]*(tri[i,(edge+1)%3]-tri[i,edge])
            nearest[(asset,*xy)].append({'distance_px':float(dist[i]**.5),'nearest_screen':p[i].tolist(),'world_yup':world.tolist(),'node':node.get('name'),'material':mat.get('name')})

def finish(rays,records,mp):
    rows=[dict(asset=k[0],pixel=list(k[1:]),nearest=min(v,key=lambda r:r['distance_px']))for k,v in nearest.items()]
    OUT.mkdir(exist_ok=False)
    (OUT/'report.json').write_text(json.dumps(dict(status='MEASURED_NEAREST_FENCE_EDGES',map_sha256=core.sha(mp),records=records,rows=rows,scope='Geometric nearest contour, alpha not applied; does not distinguish native wood from contact shadow.'),indent=2)+'\n')
    print(json.dumps(rows,indent=2))

if __name__=='__main__':
    rays=[dict(pixel=list(xy),screen=[xy[0]+.5,xy[1]+.5],hits=[])for pts in POINTS.values()for xy in pts]
    core.main(ray_records=rays,postprocess=finish,output=OUT,asset_ids=set(POINTS),triangle_callback=triangles,query_margin=5,expected_map_sha256=core.sha(core.LIB/'scenes/croisement02.rhlos-map.json'),minimum_free_bytes=10*1024**3)
