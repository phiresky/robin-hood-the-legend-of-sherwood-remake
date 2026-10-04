"""Separate the first stump's surrounding grass from its physical bark receiver."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT


def main():
    destination=OUT/'stump65-source-split-v3';destination.mkdir(exist_ok=False)
    rows=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']
    row=next(row for row in rows if row['index']==65)
    x,y=row['box_top_left'];w,h=row['box_size']
    native=np.asarray(Image.open(OUT/'baseline/masks'/row['png']).convert('L'))>0
    core=Image.new('L',(w,h));draw=ImageDraw.Draw(core)
    # The visible cap is retained in full. Below it, the dark bark silhouette
    # is bounded independently of the much wider foreground grass mask.
    polygon=[(558,653),(612,653),(612,675),(605,675),(605,684),
             (602,694),(596,702),(588,706),(578,702),(576,691),(574,681),
             (568,675),(558,675)]
    draw.polygon([(px-x,py-y) for px,py in polygon],fill=255)
    wood=native&(np.asarray(core)>0);grass=native&~wood
    assert not np.any(wood&grass) and np.array_equal(wood|grass,native)
    source=Image.open(OUT/'baseline/covered.png').convert('RGB').crop((x,y,x+w,y+h))
    for name,mask in [('wood',wood),('grass',grass)]:
        Image.fromarray(mask.astype('uint8')*255).save(destination/(name+'-domain.png'))
        rgba=source.convert('RGBA');rgba.putalpha(Image.fromarray(mask.astype('uint8')*255))
        rgba.save(destination/(name+'-cutout.png'))
    grass_image=Image.open(destination/'grass-cutout.png')
    grass_image.save(destination/'complete-source.png');grass_image.save(destination/'observed-source.png')
    board=Image.new('RGB',(w*12,h*4+24),'#333333');d=ImageDraw.Draw(board)
    for i,(name,im) in enumerate([('Native domain',source),('Wood receiver',Image.open(destination/'wood-cutout.png')),('Foreground grass',grass_image)]):
        d.text((i*w*4+3,4),name,fill='white');scaled=im.convert('RGBA').resize((w*4,h*4),Image.Resampling.NEAREST)
        board.paste(scaled,(i*w*4,24),scaled)
    board.save(destination/'partition-review.png')
    packet=dict(directory=str(destination),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=65,
                ground_z=0,observed_pixels=int(grass.sum()),source_node='foliage-southwest-stump-grass',
                inference='Hidden blade depth and root distribution inferred; RGB comes only from this native grass domain.')
    (destination/'grass-packet.json').write_text(json.dumps(packet,indent=2)+'\n')
    (destination/'partition.json').write_text(json.dumps(dict(native_mask=65,native_pixels=int(native.sum()),
        wood_pixels=int(wood.sum()),grass_pixels=int(grass.sum()),overlap_pixels=0,unassigned_pixels=0,
        wood_boundary_pixels=polygon,status='source partition candidate; visual and native-view joint review required'),indent=2)+'\n')
    print(destination)


if __name__=='__main__':main()
