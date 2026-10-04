"""Prepare conservative source-only partitions for mixed foliage75/91."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw,ImageFilter
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def main():
    out=OUT/'mixed-wood-audit/east-followup';out.mkdir(exist_ok=True)
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['masks'];source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')
    def mask(index):
        layer=Image.new('L',source.size);layer.paste(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'),tuple(native[index]['box_top_left']));return np.asarray(layer)>0
    records=[]
    for index,domain,priors in [(75,487,[131,35,38]),(91,501,[128,43,45,46])]:
        original=mask(index);remaining=original.copy();prior=np.zeros_like(original);owners=[]
        for other in priors:
            pixels=remaining&mask(other);remaining &= ~pixels;prior |= pixels;owners.append(dict(native=other,pixels=int(pixels.sum()),sha256=sha(OUT/f'baseline/masks/{other:06}.png')))
        if index==75:
            path=OUT/'missing-fence-candidates/v9/domain-431.png';pixels=remaining&(np.asarray(Image.open(path).convert('L'))>0);remaining &= ~pixels;prior |= pixels;owners.append(dict(domain=431,pixels=int(pixels.sum()),path=str(path),sha256=sha(path)))
        ground=np.zeros_like(original);uncertain=np.zeros_like(original)
        if index==75:
            # Ground boundary follows the visible litter bank behind the rails;
            # the two-pixel trace fringe remains uncertain rather than foliage.
            polygon=[(1561,729),(1567,715),(1573,705),(1580,704),(1588,711),(1602,708),(1610,715),(1636,724),(1636,749),(1536,749),(1536,731),(1558,733)]
            trace=Image.new('L',source.size);ImageDraw.Draw(trace).polygon(polygon,fill=255)
            inner=np.asarray(trace.filter(ImageFilter.MinFilter(5)))>0;outer=np.asarray(trace.filter(ImageFilter.MaxFilter(5)))>0
            ground=remaining&inner;uncertain=remaining&outer&~inner;remaining &= ~outer
        if index==75:
            # Isolated lowest tan flecks lack evidence of leaf ownership.
            lower=remaining.copy();lower[:720,:]=False;uncertain |= lower;remaining &= ~lower
        wood_boundary=mask(38) if index==75 else mask(43)|mask(45)|mask(46)
        if index==91:
            # A dark lower-left strip follows a branch-shaped silhouette;
            # imagery does not justify confidently assigning it as leaves.
            reserve=Image.new('L',source.size);ImageDraw.Draw(reserve).polygon([(1313,1004),(1336,1001),(1350,1014),(1350,1057),(1336,1067),(1317,1047)],fill=255)
            ambiguous=remaining&(np.asarray(reserve)>0);uncertain |= ambiguous;remaining &= ~ambiguous
        near=np.asarray(Image.fromarray(wood_boundary.astype('uint8')*255).filter(ImageFilter.MaxFilter(3)))>0
        uncertain |= remaining&near;remaining &= ~near
        for label,array in [('foliage',remaining),('ground',ground),('uncertain',uncertain),('prior-mixed-owners',prior)]:
            path=out/f'{index}-{label}.png';Image.fromarray(array.astype('uint8')*255).save(path)
            cut=source.copy();cut.putalpha(Image.fromarray(array.astype('uint8')*255));bg=Image.new('RGBA',source.size,(80,80,80,255));bg.alpha_composite(cut);x,y=native[index]['box_top_left'];w,h=native[index]['box_size'];bg.crop((x,y,x+w,y+h)).resize((w*5,h*5),Image.Resampling.NEAREST).save(out/f'{index}-{label}-review.png')
        if not np.array_equal(original,remaining|ground|uncertain|prior):raise ValueError('Incomplete partition')
        records.append(dict(native=index,reserved_foliage_domain=domain,foliage_path=str(out/f'{index}-foliage.png'),foliage_sha256=sha(out/f'{index}-foliage.png'),native_pixels=int(original.sum()),foliage_pixels=int(remaining.sum()),ground_pixels=int(ground.sum()),uncertain_pixels=int(uncertain.sum()),prior_mixed_pixels=int(prior.sum()),prior_owners=owners,limitations=['Prior tree masks contain interleaved leaf pixels; excluded overlap is existing mixed-owner territory, not proof every pixel is bark.','Uncertain boundary is reserved, not assigned to foliage, wood, or ground.','This source-only proposal does not mutate catalog ownership or assert missing geometry.']))
    write_json(out/'source-splits.json',dict(status='private conservative source split; peer review pending',source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),records=records))

if __name__=='__main__':main()
