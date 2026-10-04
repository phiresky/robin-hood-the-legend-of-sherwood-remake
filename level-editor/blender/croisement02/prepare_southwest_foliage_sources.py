"""Freeze southwest leaf complements from native masks and explicit shared-leaf ownership."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
SPECS={77:(488,[]),78:(489,[]),83:(493,[84,130]),84:(494,[130])}

def main():
    destination=OUT/'understory-candidates/southwest-source-v1';destination.mkdir(exist_ok=False)
    baseline=OUT/'baseline/masks';manifest=baseline/'manifest.json';rows={r['index']:r for r in json.loads(manifest.read_text())['masks']}
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    def mask(i):
        row=rows[i];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(baseline/row['png']).convert('L'))>127;return a
    records=[];domains=[];sheet=Image.new('RGB',(2400,1200),'#555555');draw=ImageDraw.Draw(sheet)
    for column,(native,(domain,exclusions)) in enumerate(SPECS.items()):
        complete=mask(native);observed=complete.copy();removed=[]
        for index in exclusions:
            cut=mask(index);removed.append(dict(native_mask=index,removed_pixels=int((observed&cut).sum()),native_mask_sha256=sha(baseline/rows[index]['png'])));observed&=~cut
        path=destination/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path);domains.append(observed)
        row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];cut=rgb[y:y+h,x:x+w].copy();cut[:,:,3]=observed[y:y+h,x:x+w]*255
        Image.fromarray(cut).save(destination/f'{native}-observed.png')
        for line,a in enumerate((complete,observed)):
            view=rgb[y:y+h,x:x+w].copy();view[:,:,3]=a[y:y+h,x:x+w]*255;image=Image.fromarray(view).resize((w*2,h*2),Image.Resampling.NEAREST);sheet.paste(image,(column*600,line*600+40),image)
            draw.text((column*600+5,line*600+5),f'{native}: '+('native mixed mask' if line==0 else f'proposed leaf domain{domain}, {observed.sum()} pixels'),fill='white')
        records.append(dict(native_mask=native,domain=domain,domain_path=str(path),domain_sha256=sha(path),native_bbox=[x,y,w,h],native_pixels=int(complete.sum()),observed_pixels=int(observed.sum()),exclusions=removed,known_rgb_changed=0,status='source proposal; physical support and state-neighbour review pending'))
    duplicate=sum(int((a&b).sum()) for i,a in enumerate(domains) for b in domains[i+1:])
    if duplicate:raise ValueError('Proposed leaf domains duplicate source pixels')
    sheet.save(destination/'source-review.png')
    write_json(destination/'source-review.json',dict(status='Proposed leaf ownership; no catalog or model changes',source_sha256=sha(source),native_manifest_sha256=sha(manifest),catalog_sha256=sha(reviewed_catalog()),records=records,duplicate_pixels=0,sheet_sha256=sha(destination/'source-review.png'),decisions=['Native77 is foreground green leaves over wattle100; native fence-mask overlap is retained as leaf source, not assumed wood.','Native78 upper overlap103 visibly contains leaves; preserve all native78. The dark lower-left fragment remains observed vegetation, with no separate trunk claimed.','Native83/84 shared1181 pixels assigned to sparse foreground84, a grouping hypothesis preserving exact identical RGB. Existing canopy130 remains its current owner.','Native83 overlaps29/30 and rock52 with visible leaves; source rock ownership already excludes native83, so those121 pixels are retained.'],pending=['Infer native77 below the south map boundary, preserve native observed RGB.','Use a multi-lobe construction for83 rather than a single inflated sphere.','Review77/78 with existing fence/logs and83/84 with approvedtrees29/30 and revised southwest rock before readiness.']))
    print(destination)
if __name__=='__main__':main()
