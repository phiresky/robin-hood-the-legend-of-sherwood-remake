"""Conservative native-texture correspondence evidence, never fabricated rigid identities."""
import argparse
import json
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import gaussian_filter,maximum_filter,binary_erosion
from scipy.signal import correlate2d
from catalog import OUT
from native_log_foreground_reference import sha


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=str,default='log-motion-correspondence-v1');parser.add_argument('--radius',type=int,default=4);parser.add_argument('--search',type=int,default=20);parser.add_argument('--features',type=int,default=100);args=parser.parse_args();assert 2<=args.radius<=8 and 1<=args.search<=60 and 1<=args.features<=1000
    source=OUT/'state-target-evidence/log-trap/full-motion';dest=OUT/args.output;dest.mkdir(exist_ok=False)
    manifest=json.loads((source/'manifest.json').read_text());records=manifest['records'];images=[np.array(Image.open(source/r['image']).convert('RGBA'))for r in records]
    grays=[p[:,:,:3].astype(float)@np.array([.299,.587,.114])/255 for p in images];alphas=[p[:,:,3]>0 for p in images];radius=args.radius;patch=radius*2+1;search=args.search
    def features(gray,alpha):
        gy,gx=np.gradient(gray);a=gaussian_filter(gx*gx,1);b=gaussian_filter(gx*gy,1);c=gaussian_filter(gy*gy,1);score=a*c-b*b-.04*(a+c)**2;valid=binary_erosion(alpha,iterations=radius+1);score[~valid]=0;peaks=(score==maximum_filter(score,size=7))&(score>1e-7);ys,xs=np.nonzero(peaks);order=np.argsort(score[ys,xs])[::-1][:args.features];return [(int(xs[i]),int(ys[i]))for i in order]
    def match(a,b,ab,bb,x,y):
        h,w=a.shape
        if x<radius or y<radius or x>=w-radius or y>=h-radius:return None
        template=a[y-radius:y+radius+1,x-radius:x+radius+1];centered=template-template.mean();den=np.sqrt(np.sum(centered**2))
        if den<.04:return None
        left=max(radius,x-search);right=min(w-radius-1,x+search);top=max(radius,y-search);bottom=min(h-radius-1,y+search)
        region=b[top-radius:bottom+radius+1,left-radius:right+radius+1];ones=np.ones((patch,patch));sums=correlate2d(region,ones,mode='valid');squares=correlate2d(region*region,ones,mode='valid');std=np.sqrt(np.maximum(squares-sums*sums/(patch*patch),1e-10));scores=correlate2d(region,centered,mode='valid')/(std*den)
        opaque=correlate2d(bb[top-radius:bottom+radius+1,left-radius:right+radius+1].astype(float),ones,mode='valid')/(patch*patch);scores[opaque<.9]=-1
        yy,xx=np.unravel_index(np.argmax(scores),scores.shape);best=float(scores[yy,xx]);others=scores.copy();others[max(0,yy-3):yy+4,max(0,xx-3):xx+4]=-1;gap=best-float(others.max())
        if best<.85 or gap<.035:return None
        return left+int(xx),top+int(yy),best,gap
    pairs=[];sheet=Image.new('RGB',(256*5,265*6),'#222')
    for index in range(len(records)-1):
        a,b=grays[index:index+2];aa,bb=alphas[index:index+2];points=features(a,aa);matches=[]
        preview=Image.fromarray(images[index+1]);bg=Image.new('RGBA',preview.size,(30,30,30,255));bg.alpha_composite(preview);draw=ImageDraw.Draw(bg)
        for x,y in points:
            found=match(a,b,aa,bb,x,y)
            if found is None:continue
            nx,ny,score,gap=found;back=match(b,a,bb,aa,nx,ny)
            if back is None or np.hypot(back[0]-x,back[1]-y)>1:continue
            matches.append(dict(source=[x,y],target=[nx,ny],ncc=score,uniqueness_gap=gap,roundtrip_error=float(np.hypot(back[0]-x,back[1]-y))))
            draw.line((x,y,nx,ny),fill=(255,40,240),width=1);draw.ellipse((nx-1,ny-1,nx+1,ny+1),fill=(40,255,100))
        entry=dict(first_tick=records[index]['first_tick'],next_tick=records[index+1]['first_tick'],features=len(points),accepted=len(matches),matches=matches);pairs.append(entry)
        bg.save(dest/f'{index:02d}.png');sheet.paste(bg.convert('RGB'),((index%5)*256,(index//5)*265+22));ImageDraw.Draw(sheet).text(((index%5)*256+4,(index//5)*265+4),f"{entry['first_tick']}→{entry['next_tick']}: {len(matches)}/{len(points)}",fill='white')
    sheet.save(dest/'correspondence-sheet.png')
    result=dict(status='source correspondence diagnostic; not rigid-log identity or3Dmotion',source_manifest_sha256=sha(source/'manifest.json'),source_frames=[dict(image=r['image'],sha256=sha(source/r['image']))for r in records],parameters=dict(patch=patch,search=search,features=args.features,ncc_min=.85,uniqueness_gap_min=.035,roundtrip_max=1),pairs=pairs,limitations=['Repeated bark can still produce ambiguous matches; source evidence needs visual and rigid-component validation.','Occlusion holes are native composition, not physical fractures.','No new log count, depth, texture, geometry or permanent state changes inferred.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print([(p['first_tick'],p['accepted'],p['features'])for p in pairs])

if __name__=='__main__':main()
