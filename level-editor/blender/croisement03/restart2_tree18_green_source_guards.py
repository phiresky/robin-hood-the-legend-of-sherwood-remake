"""Independent source-center raster audit for inferred regional plant geometry."""
import json
import hashlib
import math
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
B=Path(__file__).resolve().parents[3]/'level-editor/work/croisement03-refinement/restart2'


def main():
    folder=B/'tree18-regional-green-cpu-v2';gp=folder/'geometry.json';d=json.loads(gp.read_text());rp=B/'tree18-regional-plants-source-v2/receipt.json';r=json.loads(rp.read_text());bp=B/'bank-coupled-left-full-source-v3/projection.json';bank=json.loads(bp.read_text());wp=B/'tree18-priority-source-proposals-v1/receipt.json';wood=json.loads(wp.read_text())
    layers={name:Image.new('1',(1408,960))for name in ['reserved','known_bank_stone','proposed_bark']};draw=ImageDraw.Draw(layers['reserved'])
    for opening in r['reserved_openings_and_context']:
        if 'outline'in opening:draw.polygon([tuple(q)for q in opening['outline']],fill=1)
        else:draw.line([tuple(q)for q in opening['line']],width=opening['review_width_px'],fill=1)
    known={tuple(q)for q in bank['accepted_pixels']}|{tuple(q[:2])for q in bank['protected_firsthits']};assert len(known)==29000
    draw=ImageDraw.Draw(layers['known_bank_stone'])
    for q in known:draw.point(q,fill=1)
    draw=ImageDraw.Draw(layers['proposed_bark'])
    for q in wood['regions'][0]['exact_source_pixels_xy']:draw.point(tuple(q),fill=1)
    nativepath=B/'tree18-regional-plants-source-v1/native-union-priority-only.png';native=np.array(Image.open(nativepath));native=native[:,:,3]>0 if native.ndim==3 and native.shape[2]==4 else native>0
    if native.ndim==3:native=np.any(native,axis=2)
    masks={k:np.array(v)for k,v in layers.items()};masks['native']=native;S,C=math.sin(math.radians(35)),math.cos(math.radians(35));results=[]
    for part in d['parts']:
        vertices=np.array(part['vertices']);xy=np.stack((vertices[:,0],-vertices[:,1]*S-vertices[:,2]*C),axis=1);covered=set()
        for face in part['faces']:
            t=xy[face];lo=np.floor(t.min(0)).astype(int);hi=np.ceil(t.max(0)).astype(int);a,b=t[1]-t[0],t[2]-t[0];det=a[0]*b[1]-a[1]*b[0]
            if abs(det)<1e-10:continue
            for y in range(max(0,lo[1]),min(960,hi[1]+1)):
                for x in range(max(0,lo[0]),min(1408,hi[0]+1)):
                    q=np.array([x+.5,y+.5])-t[0];u=(q[0]*b[1]-q[1]*b[0])/det;v=(a[0]*q[1]-a[1]*q[0])/det
                    if min(u,v,1-u-v)>=-1e-8:covered.add((x,y))
        results.append({'part':part['id'],'projected_pixels':len(covered),'overlaps':{k:[list(q)for q in sorted(covered)if mask[q[1],q[0]]]for k,mask in masks.items()}})
    paths=[gp,rp,bp,wp,nativepath,Path(__file__)];receipt={'status':'PASS_SCOPED_PROJECTED_EXCLUSIONS'if all(not pixels for row in results for pixels in row['overlaps'].values())else'HOLD_SOURCE_DOMAIN_OVERLAP','pins':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths},'parts':results,'known_bank_pixels':len(known),'limits':['Exact source-center triangle projection is independent of clump-center admission. Geometry-only test, not source material ownership.','Intersection with bank2D domain is conservative: actual saved depth may place green behind stone. Retain all reported pixels for jointfirst-hit resolution.']};output=folder/'independent-full-source-guards.json';assert not output.exists();output.write_text(json.dumps(receipt,indent=2)+'\n');print(receipt['status'],[(r['part'],{k:len(v)for k,v in r['overlaps'].items()})for r in results])


if __name__=='__main__':main()
