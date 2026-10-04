"""Freeze eastern and southern leaf-only source proposals independently of mixed wood domains."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
SPECS={74:(486,[]),85:(495,[128]),86:(496,[]),87:(497,[132]),88:(498,[]),89:(499,[128]),90:(500,[128])}

def main():
    destination=OUT/'understory-candidates/east-south-source-v1';destination.mkdir(exist_ok=False)
    baseline=OUT/'baseline/masks';manifest=baseline/'manifest.json';rows={r['index']:r for r in json.loads(manifest.read_text())['masks']}
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    def mask(i):
        row=rows[i];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(baseline/row['png']).convert('L'))>127;return a
    records=[];domains=[];sheet=Image.new('RGB',(2100,1000),'#555555');draw=ImageDraw.Draw(sheet)
    for column,(native,(domain,exclusions)) in enumerate(SPECS.items()):
        complete=mask(native);observed=complete.copy();removed=[]
        for index in exclusions:
            cut=mask(index);removed.append(dict(native_mask=index,removed_pixels=int((observed&cut).sum()),native_mask_sha256=sha(baseline/rows[index]['png'])));observed&=~cut
        path=destination/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path);domains.append(observed)
        row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];cut=rgb[y:y+h,x:x+w].copy();cut[:,:,3]=observed[y:y+h,x:x+w]*255
        Image.fromarray(cut).save(destination/f'{native}-observed.png')
        for line,a in enumerate((complete,observed)):
            view=rgb[y:y+h,x:x+w].copy();view[:,:,3]=a[y:y+h,x:x+w]*255;image=Image.fromarray(view).resize((w,h),Image.Resampling.NEAREST);sheet.paste(image,(column*300,line*500+40),image)
            draw.text((column*300+5,line*500+5),f'{native}: '+('native mixed mask' if line==0 else f'proposed leaf domain{domain}, {observed.sum()} pixels'),fill='white')
        records.append(dict(native_mask=native,domain=domain,domain_path=str(path),domain_sha256=sha(path),native_bbox=[x,y,w,h],native_pixels=int(complete.sum()),observed_pixels=int(observed.sum()),exclusions=removed,known_rgb_changed=0,status='source proposal; physical support and state-neighbour review pending'))
    duplicate=sum(int((a&b).sum()) for i,a in enumerate(domains) for b in domains[i+1:])
    if duplicate:raise ValueError('Proposed leaf domains duplicate source pixels')
    sheet.save(destination/'source-review.png')
    write_json(destination/'source-review.json',dict(status='Proposed leaf ownership; no catalog or model changes',source_sha256=sha(source),native_manifest_sha256=sha(manifest),catalog_sha256=sha(reviewed_catalog()),records=records,duplicate_pixels=0,sheet_sha256=sha(destination/'source-review.png'),decisions=['Leaf-only masks74/86/88 retain visible vegetation even where native wall96, logs108, tree47 or shed127 gameplay masks overlap. Those cached owners need source-role exclusions at eventual registration, not geometry edits.','Native85 retains confirmed675 leaf pixels over oak35. Existing canopy128 remains its current owner; unique93 complement already excludes85.','Native87 lower straw-like fringe is part of this vegetation; existing animated canopy132 is excluded. Native39/40 overlaps do not expose substantial trunk bark in the inspected cutout.','Native89 has separate south-edge masses; preserve individual clumps and infer continuation below the map rather than one large volume. Native90 is a small grass/shrub clump.','Mixed75 and91 intentionally remain separate pending exact bark/fence/ground splits. No blanket vegetation inference from mask type.'],pending=['Actual multi-view geometry and existing-neighbour joints required.','Observed ownership transfer must preserve prior models and document cached material contamination separately.','Infer east boundary87 and south boundary89 completion; no user approval implied.']))
    print(destination)
if __name__=='__main__':main()
