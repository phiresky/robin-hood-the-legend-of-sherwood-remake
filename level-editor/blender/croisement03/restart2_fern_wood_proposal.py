"""Expose a conservative visible-bark proposal without altering fern candidates."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement03-refinement/baseline'
OUT=BASE.parent/'restart2/fern-wood-proposal-v1'
SPECS={35:dict(wood=13,polygon=[(1045,64),(1050,64),(1048,77),(1048,88),(1044,91),(1043,78)]),76:dict(wood=12,polygon=[(1005,142),(1009,142),(1008,152),(1007,158),(1007,168),(1003,171),(1003,159),(1004,153)])}

def main():
    OUT.mkdir(parents=True,exist_ok=False);source=Image.open(BASE/'covered.png').convert('RGB');rgb=np.array(source).astype(int);level=json.loads((BASE/'Croisement03.rhp.json').read_text());reports=[]
    def domain(index):
        row=level['masks'][index];image=Image.new('L',source.size);image.paste(Image.open(BASE/f'masks/{index:06}.png'),tuple(row['box_top_left']));return np.array(image)>0
    for native,spec in SPECS.items():
        region=Image.new('L',source.size);ImageDraw.Draw(region).polygon(spec['polygon'],fill=255)
        overlap=domain(native)&domain(spec['wood']);selection=overlap&(np.array(region)>0)&(rgb[:,:,0]>=rgb[:,:,1]+5)
        path=OUT/f'{native}-proposed-wood.png';Image.fromarray(selection.astype('uint8')*255).save(path)
        x,y=level['masks'][native]['box_top_left'];w,h=level['masks'][native]['box_size'];box=(x-6,y-6,x+w+6,y+h+6)
        marked=np.array(source);marked[selection]=[255,0,255]
        raw=source.crop(box).resize(((w+12)*6,(h+12)*6),Image.Resampling.NEAREST);overlay=Image.fromarray(marked).crop(box).resize(raw.size,Image.Resampling.NEAREST)
        sheet=Image.new('RGB',(raw.width*2,raw.height));sheet.paste(raw,(0,0));sheet.paste(overlay,(raw.width,0));sheet.save(OUT/f'{native}-proposal-comparison.png')
        reports.append(dict(fern_mask=native,wood_mask=spec['wood'],trace_polygon=spec['polygon'],selection_pixels=int(selection.sum()),total_overlap=int(overlap.sum()),selection_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),status='PROPOSAL ONLY: native warm-bark colors within an independently traced visible trunk strip; not a global color classifier',remaining='Green and ambiguous overlap is deliberately unresolved, not reassigned automatically. No model, alpha, approval or canonical manifest changed.'))
    (OUT/'proposal.json').write_text(json.dumps(dict(source_sha256=hashlib.sha256((BASE/'covered.png').read_bytes()).hexdigest(),items=reports),indent=2)+'\n');print([(r['fern_mask'],r['selection_pixels']) for r in reports])

if __name__=='__main__':main()
