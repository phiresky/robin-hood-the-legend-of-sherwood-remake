"""Separate visible western rocks from overlapping native vegetation domains."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from evidence_io import sha, write_json

# Conservative lower rock boundary traced on the native 280x180 source crop.
# The native mask supplies the detailed exterior; foliage retains native alpha.
LOWER_BOUNDARY = [(0,280),(279,280),(279,401),(254,401),(235,410),
    (210,409),(192,401),(183,399),(172,413),(157,420),(134,422),
    (117,404),(104,393),(85,382),(65,355),(40,354),(0,354)]


def main():
    destination = OUT / 'west-rock-source-revision'
    destination.mkdir(exist_ok=True)
    inventory = json.loads((OUT / 'scenery-domains/inventory.json').read_text())
    rows = {r['index']: r for r in inventory['masks']}
    def canvas(index):
        row = rows[index]
        x,y = row['box_top_left']; w,h = row['box_size']
        result = np.zeros((1152,1792), bool)
        result[y:y+h,x:x+w] = np.asarray(Image.open(row['png']).convert('L')) > 0
        return result
    trace = Image.new('L',(1792,1152))
    ImageDraw.Draw(trace).polygon(LOWER_BOUNDARY, fill=255)
    foreground = canvas(56) | canvas(61)
    rock = canvas(49) & ~foreground & (np.asarray(trace)>0)
    domains = {350:rock,351:canvas(57)&~rock,352:canvas(60)&~rock,354:~rock}
    for index, domain in domains.items():
        image = destination / f'domain-{index}.png'
        Image.fromarray(domain.astype('uint8')*255).save(image)
        inventory['masks'].append(dict(index=index,layer=0,png=str(image),
            box_top_left=[0,0],box_size=[1792,1152],
            provenance='Reviewed rock/vegetation split; see ownership-review.json'))
    write_json(destination/'inventory.json',inventory)
    manifest = json.loads((OUT/'scenery-round-2/assets/croisement02-west-rock-outcrop/source-masks.json').read_text())
    manifest['mask_inventory'] = str(destination/'inventory.json')
    assignment = next(r for r in manifest['projections']['exterior']['assignments']
                      if r.get('asset_group')=='croisement02-west-rock-outcrop')
    assignment['mask_indices'] = [350]
    for key in ('exclude_mask_indices','exclusions_reviewed','exclusion_reason'):
        assignment.pop(key,None)
    receivers = [f'building-{i:03}' for i in range(37,43)]
    manifest['projections']['exterior']['occluder_constraints'] = [dict(
        reviewed=True,source_node=node,receiver_nodes=receivers,mask_indices=[354],
        reason='The reviewed visible-rock domain belongs to these rocks. Unrelated coarse context proxies may occlude only outside that observed domain.',
        review_evidence=str(destination/'ownership-review.json'))
        for node in ['ground',*(f'building-{i:03}' for i in range(150))]
        if node not in receivers]
    write_json(destination/'assignments.json',manifest)
    source = OUT/'animation-references/composite-frame-0.png'
    rgba = Image.open(source).convert('RGBA')
    sheet = Image.new('RGB',(1120,4*744),'#aaa')
    draw = ImageDraw.Draw(sheet)
    for index,(name,domain) in enumerate([('Native source context',None),
            ('Rock domain: native49 minus56/61 and lower terrain',rock),
            ('Native57 remainder; rock removed',domains[351]),
            ('Native60 remainder; rock removed',domains[352])]):
        im = rgba.copy()
        if domain is not None: im.putalpha(Image.fromarray(domain.astype('uint8')*255))
        im = im.crop((0,280,280,460)).resize((1120,720),Image.Resampling.NEAREST)
        draw.text((4,index*744+4),name,fill='black')
        sheet.paste(im,(0,index*744+24),im)
    sheet.save(destination/'ownership-sheet.png')
    write_json(destination/'ownership-review.json',dict(status='source split candidate; visual review required',
        source_sha256=sha(source),sheet_sha256=sha(destination/'ownership-sheet.png'),
        domain_hashes={str(i):sha(destination/f'domain-{i}.png') for i in domains},
        lower_boundary=LOWER_BOUNDARY, rock_pixels=int(rock.sum()),
        removed_57_rock_pixels=int((canvas(57)&rock).sum()),
        removed_60_rock_pixels=int((canvas(60)&rock).sum()),
        notes=['Native49 includes both rocks and foreground plants. Native56/61 provide the foreground silhouettes.',
               'Native57/60 contain rock pixels also present in49; their remainder is retained separately, never projected onto rock.',
               'Lower sparse ground between bushes is excluded with a conservative source-coordinate trace.',
               'Rock self-occlusion is retained; coarse neighboring proxy volumes do not define source-visible rock ownership.']))
    print(destination)


if __name__=='__main__': main()
