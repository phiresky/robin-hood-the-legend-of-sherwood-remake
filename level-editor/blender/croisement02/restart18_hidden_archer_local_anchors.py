"""Survey short inferred rock attachments behind immutable native leaf pixels."""
import json
import numpy as np
from scipy.ndimage import label
from PIL import Image
from restart18_hidden_archer_route_cpu import BASE,RockDepth,SIN,RAY,sha
DEST=BASE/'climbing-v17/local-attachments-cpu-v1'
def main():
    assert not DEST.exists();DEST.mkdir();sp=BASE/'surface-v8/surfaces.npz';d=np.load(sp);rock=RockDepth(d['vertices0'][d['triangles0']]);records=[];bindings={str(sp):sha(sp)}
    for state in ['initial','applied']:
        pp=BASE/f'skeleton-v9-cpu/{state}-plan.json';ap=BASE/f'lobes-v16-cpu/{state}-arrangement.json';bindings[str(pp)]=sha(pp);bindings[str(ap)]=sha(ap);plan=json.loads(pp.read_text());arr=json.loads(ap.read_text());src=Image.open(plan['source']).convert('RGBA');alpha=np.array(src)[:,:,3]>=128;components,n=label(alpha,np.ones((3,3)));yy,xx=np.where(alpha);ox,oy=plan['source_top_left'];xy=np.column_stack([xx+ox+.5,yy+oy+.5]);starts=np.column_stack([xy[:,0],-xy[:,1]/SIN,np.zeros(len(xx))]);front=np.full(len(xx),-np.inf)
        for l in arr['lobes']:
            inverse=np.linalg.inv(l['axes']);q=(starts-np.array(l['center']))@inverse.T;v=RAY@inverse.T;a=v@v;b=2*(q@v);c=(q*q).sum(1)-1;disc=b*b-4*a*c;ok=disc>=0;t=np.full(len(xx),-np.inf);t[ok]=(-b[ok]+np.sqrt(disc[ok]))/(2*a);front=np.maximum(front,t)
        rockdepth=rock.front(xy);leafdepth=starts@RAY+front;gap=leafdepth-rockdepth;usable=np.isfinite(rockdepth)&(gap>1.8)&(gap<12);rows=[]
        for ci in range(1,n+1):
            indices=np.flatnonzero(components[yy,xx]==ci);good=indices[usable[indices]];order=good[np.argsort(gap[good])];chosen=order[:8];rows.append(dict(component=ci,pixels=len(indices),short_attachment_candidates=len(good),anchors=[dict(source_center=xy[i].tolist(),estimated_leaf_world=(starts[i]+RAY*front[i]).tolist(),rock_surface_world=(starts[i]+RAY*(rockdepth[i]-starts[i]@RAY)).tolist(),estimated_leaf_to_rock=gap[i]) for i in chosen]))
        records.append(dict(state=state,native_pixels=len(xx),components=n,short_attachment_candidates=int(usable.sum()),components_without_short_anchor=sum(not r['short_attachment_candidates'] for r in rows),component_anchors=rows))
    result=dict(status='CPU local attachment hypothesis; no construction approval',inputs=bindings,states=records,scope='Discard historical bank-to-crest guides. Test compact short branched supports hidden behind native leaf patches, with explicitly inferred local rock-face/crest attachments.',limitations=['Leaf depths are unjittered envelope hits; exact saved leaf surfaces and pre-save corrections must be extracted before construction.','Rock surface points are geometric hits; crevice attachment and receiver opacity are inferred and need contextual review.','A ray-aligned attachment is source-hidden at its center only; full tube silhouette and triangle clearance are not yet proven.','Unanchored components need compact neighboring branches or separately justified inferred attachment; no unsupported lobes may be retained.'])
    (DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps([{k:v for k,v in s.items() if k!='component_anchors'} for s in records]))
if __name__=='__main__':main()
