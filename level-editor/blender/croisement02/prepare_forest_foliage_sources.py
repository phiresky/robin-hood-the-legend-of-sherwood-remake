"""Freeze proposed forest foliage domains while preserving wood and state covers."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
SPECS={62:(483,[63,133,137]),63:(484,[130,133,136]),64:(485,[0,1,3,55,133])}

def main():
    destination=OUT/'understory-candidates/forest-source-v1';destination.mkdir(exist_ok=False)
    baseline=OUT/'baseline/masks';manifest=baseline/'manifest.json';rows={r['index']:r for r in json.loads(manifest.read_text())['masks']}
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    def mask(i):
        row=rows[i];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(baseline/row['png']).convert('L'))>127;return a
    records=[];domains=[];sheet=Image.new('RGB',(1800,960),'#555555');draw=ImageDraw.Draw(sheet)
    for column,(native,(domain,exclusions)) in enumerate(SPECS.items()):
        complete=mask(native);observed=complete.copy();removed=[]
        for index in exclusions:
            cut=mask(index);removed.append(dict(native_mask=index,removed_pixels=int((observed&cut).sum()),native_mask_sha256=sha(baseline/rows[index]['png'])));observed&=~cut
        path=destination/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path);domains.append(observed)
        row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];cut=rgb[y:y+h,x:x+w].copy();cut[:,:,3]=observed[y:y+h,x:x+w]*255
        Image.fromarray(cut).save(destination/f'{native}-observed.png')
        for line,a in enumerate((complete,observed)):
            view=rgb[y:y+h,x:x+w].copy();view[:,:,3]=a[y:y+h,x:x+w]*255;image=Image.fromarray(view).resize((w*3,h*3),Image.Resampling.NEAREST);sheet.paste(image,(column*600,line*480+40),image)
            draw.text((column*600+5,line*480+5),f'{native}: '+('native mixed mask' if line==0 else f'proposed leaf domain{domain}, {observed.sum()} pixels'),fill='white')
        records.append(dict(native_mask=native,domain=domain,domain_path=str(path),domain_sha256=sha(path),native_bbox=[x,y,w,h],native_pixels=int(complete.sum()),observed_pixels=int(observed.sum()),exclusions=removed,known_rgb_changed=0,status='source proposal; physical support and state-neighbour review pending'))
    duplicate=sum(int((a&b).sum()) for i,a in enumerate(domains) for b in domains[i+1:])
    if duplicate:raise ValueError('Proposed leaf domains duplicate source pixels')
    sheet.save(destination/'source-review.png')
    write_json(destination/'source-review.json',dict(status='Proposed leaf ownership; no catalog or model changes',source_sha256=sha(source),native_manifest_sha256=sha(manifest),catalog_sha256=sha(reviewed_catalog()),records=records,duplicate_pixels=0,sheet_sha256=sha(destination/'source-review.png'),decisions=['Native62/63 visually contain green foliage over background trunks; do not subtract their wood-mask overlaps blindly.','Shared62/63 artwork is assigned to63 as an explicit foreground grouping hypothesis; native source RGB is identical at shared pixels.','Native64 includes visible existing birch wood0/1, excluded from leaf ownership along with existing55 and canopy133.','Global136(layer-local135) is the initial log trap and global137(layer-local136) is the initial rock trap, independently confirmed by state review; their artwork and existing canopies retain ownership.'],pending=['Solve actual support against selected bank0–4; these clumps may stand above the ground datum.','Inspect each clump with current approved neighbouring trees and the bank before readiness.']))
    print(destination)
if __name__=='__main__':main()
