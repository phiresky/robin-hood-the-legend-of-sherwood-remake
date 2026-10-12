"""Show native mask pixels against raw Day artwork; does not infer ownership."""
import hashlib,json
from pathlib import Path
from PIL import Image,ImageDraw,ImageFont

EDITOR=Path(__file__).resolve().parents[2]
ROOT=EDITOR/'work/needs-review/sherwood-refinement/textures/native-mask-audit'
INVENTORY=EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.d/masks/manifest.json'
SOURCE=EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'


def main():
    ROOT.mkdir(parents=True,exist_ok=True)
    masks=json.loads(INVENTORY.read_text())['masks'];art=Image.open(SOURCE).convert('RGB');font=ImageFont.load_default(size=17);pages=[]
    for start in range(0,len(masks),12):
        batch=masks[start:start+12];page=Image.new('RGB',(1440,1200),'#20242b');draw=ImageDraw.Draw(page)
        for i,m in enumerate(batch):
            x,y=(i%3)*480,(i//3)*300;l,t=m['box_top_left'];w,h=m['box_size']
            crop=art.crop((l,t,l+w,t+h));mask=Image.open(INVENTORY.parent/m['png']).convert('L');assert mask.size==crop.size
            owned=Image.new('RGB',crop.size,'#10141a');owned.paste(crop,(0,0),mask)
            for col,image in enumerate((crop,owned)):
                image.thumbnail((230,265));page.paste(image,(x+col*240+(230-image.width)//2,y+30+(265-image.height)//2))
            draw.text((x+5,y+5),f"Mask {m['index']} / L{m['layer']} / native {m['obstacle_indices']}",fill='white',font=font)
        filename=f'masks-{start:03}-{start+len(batch)-1:03}.png';page.save(ROOT/filename);pages.append(dict(file=filename,indices=[m['index'] for m in batch]))
    (ROOT/'pages.json').write_text(json.dumps(dict(inventory_sha256=hashlib.sha256(INVENTORY.read_bytes()).hexdigest(),
        source_sha256=hashlib.sha256(SOURCE.read_bytes()).hexdigest(),pages=pages),indent=2)+'\n')
    print(f'Prepared {len(pages)} audit pages for {len(masks)} native masks.')


if __name__=='__main__':main()
