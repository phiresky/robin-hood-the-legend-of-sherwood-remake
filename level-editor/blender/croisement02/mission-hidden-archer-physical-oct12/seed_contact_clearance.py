"""Add observed subpixel intersection witnesses to the private clearance field."""
import json,math
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter,maximum_filter
import qualify as q

def main():
    source=q.OUT/'conformation-v1/depth-field.json'
    contacts=q.OUT/'geometry-v1-contacts/report.json'
    result=json.loads(source.read_text())
    evidence=json.loads(contacts.read_text())
    anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text())
    qualification=json.loads((q.OUT/'qualification.json').read_text())
    assert result['map']==evidence['map']
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    for number, row in result['rows'].items():
        n=int(number);a=anchors[number];box=row['box'];bounds=qualification['rows'][n-1]['source_union_box']
        field=np.array(row['camera_ray_advance']);seeds=np.zeros_like(field);used=[]
        def depth(x,y):
            t=(x-bounds[0])/(bounds[2]-bounds[0])
            wy=a['world_y_left']*(1-t)+a['world_y_right']*t+a['depth_bulge']*math.sin(math.pi*t)
            return -wy/cosine-y*sine/cosine
        for state in evidence['rows']:
            if state['profile']!=n:continue
            for w in state['witnesses']:
                x,y=w['source_cell'];corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
                assert all(box[0]<=xx<=box[2] and box[1]<=yy<=box[3] for xx,yy in corners)
                delta=max(0,w['camera_space_witness'][2]+a['thickness']+.4-min(depth(*p)for p in corners))
                for xx,yy in corners:seeds[yy-box[1],xx-box[0]]=max(seeds[yy-box[1],xx-box[0]],delta)
                used.append({'phase':state['phase'],'source_cell':[x,y],'asset':w['asset'],'node':w['node'],'advance':delta})
        required=np.maximum(field,seeds)
        field=np.maximum(required,gaussian_filter(maximum_filter(required,size=5),1.0))
        row.update(camera_ray_advance=field.tolist(),maximum_advance=float(field.max()),contact_witness_seeds=used)
    result['input_corner_field']=q.pin(source);result['contact_witness_source']=q.pin(contacts)
    result['limits'].append('Frozen subpixel alpha-aware contact witnesses seed all cell corners, including back-facing Crown surfaces. This is clearance inference, not a structural attachment or complete collision proof. The witness list is capped; recheck all receiver triangles.')
    target=q.OUT/'conformation-v2';assert not target.exists();target.mkdir()
    (target/'depth-field.json').write_text(json.dumps(result,indent=2)+'\n')
    print({n:{'max_advance':r['maximum_advance'],'contact_seeds':len(r['contact_witness_seeds'])}for n,r in result['rows'].items()})

if __name__=='__main__':main()
