"""Preserve explicit per-phase foreground conflicts in the prior bank source sample set."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/active/croisement03/restart2'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    output=B/'bank-prior-native-overlap-v1'
    assert not output.exists()
    source=B.parent/'baseline/covered.png'
    proposal=B/'bank-full-visible-stone-audit-v3/receipt.json'
    prior=B/'bank-main-masses-source-v1/projection.json'
    report=json.loads(proposal.read_text())
    old=json.loads(prior.read_text())
    samples={tuple(p[:2]) for p in old['protected_firsthits']}|{tuple(p) for p in old['accepted_pixels']}
    assert len(samples)==11257
    pins={str(p):sha(p) for p in (source,proposal,prior)}
    source_rgba=np.array(Image.open(source).convert('RGBA'))
    overlaps={}
    for row in report['native_proof']:
        animation,phase=row['animation'],row['phase']
        path=B.parent/'animation-references'/f'animation-{animation:02}'/f'{phase:03}.png'
        assert sha(path)==row['normalized_sha256']
        pins[str(path)]=sha(path)
        rgba=np.array(Image.open(path).convert('RGBA'))
        x0,y0,w,h=row['native_box_xywh']
        assert rgba.shape==(h,w,4)
        for x,y in samples:
            xx,yy=x-x0,y-y0
            if 0<=xx<w and 0<=yy<h and rgba[yy,xx,3]:
                overlaps.setdefault((x,y),[]).append({'animation':animation,'phase':phase,'native_rgba':rgba[yy,xx].tolist()})
    assert len(overlaps)==12
    rows=[{'source_xy':list(p),'old_stone_source_rgba':source_rgba[p[1],p[0]].tolist(),'native_phases':v,'resolution':'UNRESOLVED until exact saved native card/source priority and source-state provenance are checked'}for p,v in sorted(overlaps.items())]
    output.mkdir()
    receipt={'status':'EXPLICIT_PRIOR_FOREGROUND_OVERLAPS_REQUIRING_SAVED_PRIORITY_CHECK','pins':pins,'old_source_samples':11257,'overlap_pixels':12,'rows':rows,'limits':['Source coordinates intersect native alpha; this alone does not show which surface is geometrically in front.','Old projected colors and source masks remain unchanged. Native foreground must not be removed to retain these samples.','Correct source-state provenance may justify stone behind foreground; exposed duplication or leaf-colored stone cannot be accepted without review.']}
    (output/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
    points=np.array(list(overlaps));lo=points.min(0)-8;hi=points.max(0)+9;box=(*lo,*hi)
    raw=Image.fromarray(source_rgba).convert('RGB');marked=raw.copy();draw=ImageDraw.Draw(marked)
    for x,y in overlaps:draw.rectangle((x-1,y-1,x+1,y+1),outline=(255,0,200))
    a=raw.crop(box).resize(((hi[0]-lo[0])*4,(hi[1]-lo[1])*4),Image.Resampling.NEAREST);b=marked.crop(box).resize(a.size,Image.Resampling.NEAREST);sheet=Image.new('RGB',(a.width*2,a.height));sheet.paste(a,(0,0));sheet.paste(b,(a.width,0));sheet.save(output/'source-overlaps-4x.png')
    print({'pixels':len(rows),'phase_occurrences':sum(len(r['native_phases'])for r in rows)})


if __name__=='__main__':
    main()
