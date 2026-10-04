"""Write native mask crops for visual ownership classification, without assigning owners."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement03-refinement'
def main():
    level = json.loads((OUT / 'baseline/Croisement03.rhp.json').read_text())
    source = Image.open(OUT / 'baseline/covered.png').convert('RGB')
    output = OUT / 'mask-survey'; output.mkdir(exist_ok=True)
    cards = []; rows = []
    for i, mask in enumerate(level['masks']):
        x,y = mask['box_top_left']; w,h = mask['box_size']
        alpha = Image.open(OUT / f'baseline/masks/{i:06}.png').convert('L')
        crop = source.crop((x,y,x+w,y+h)); back = Image.new('RGB',crop.size,'#303030'); back.paste(crop,mask=alpha)
        back.save(output / f'{i:03}.png')
        context = source.crop((max(0,x-12),max(0,y-12),min(source.width,x+w+12),min(source.height,y+h+12)))
        card = Image.new('RGB',(360,250),'#242424'); back.thumbnail((178,218)); context.thumbnail((178,218))
        card.paste(back,((180-back.width)//2,25+(218-back.height)//2)); card.paste(context,(180+(180-context.width)//2,25+(218-context.height)//2))
        ImageDraw.Draw(card).text((6,4),f"Mask {i} / layer {mask['layer']} / type {mask['mask_type']}",fill='white')
        cards.append(card)
        rows.append(dict(index=i,layer=mask['layer'],mask_type=mask['mask_type'],box=[x,y,w,h],pixels=int(np.count_nonzero(alpha)),obstacles=mask['obstacle_indices'],classification='unreviewed'))
    for start in range(0,len(cards),16):
        sheet=Image.new('RGB',(1440,1000),'#242424')
        for n,card in enumerate(cards[start:start+16]): sheet.paste(card,((n%4)*360,(n//4)*250))
        sheet.save(output/f'sheet-{start//16:02}.jpg',quality=95)
    (output/'index.json').write_text(json.dumps(dict(source_sha256=hashlib.sha256((OUT/'baseline/covered.png').read_bytes()).hexdigest(),masks=rows),indent=2)+'\n')
    print(json.dumps(dict(masks=len(rows),sheets=(len(rows)+15)//16)))
if __name__=='__main__': main()
