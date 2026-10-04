"""Separate northwest cliff artwork from neighbouring foliage and ground."""
import json
from pathlib import Path
import sys
import numpy as np
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
OUTLINE=[(0,0),(150,0),(143,28),(139,47),(127,58),(119,73),(111,88),(108,105),(120,116),(126,129),(125,143),(120,157),(108,170),(73,178),(30,177),(0,183)]
SMALL=[(0,155),(20,160),(24,169),(18,179),(6,194),(0,195)]

def main():
    folder=OUT/'northwest-rock-source-revision';folder.mkdir(exist_ok=True)
    inventory=json.loads((OUT/'scenery-domains/inventory.json').read_text());masks={}
    for index in [0,50,51,53,54,133]:
        row=next(r for r in inventory['masks'] if r['index']==index)
        im=Image.new('L',(1792,1152));im.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']));masks[index]=np.asarray(im)>0
    clip=Image.new('L',(1792,1152));ImageDraw.Draw(clip).polygon(OUTLINE,fill=255)
    small=Image.new('L',clip.size);ImageDraw.Draw(small).polygon(SMALL,fill=255)
    domain=(((masks[50]|masks[53])&(np.asarray(clip)>0))|(masks[51]&(np.asarray(small)>0)))&~masks[54]&~masks[133]&~masks[0]
    for index,array in [(380,domain),(381,~domain)]:
        path=folder/f'domain-{index}.png';Image.fromarray(array.astype('uint8')*255).save(path)
        inventory['masks'].append(dict(index=index,png=str(path),layer=0,box_top_left=[0,0],box_size=[1792,1152],provenance='Native northwest rock artwork clipped to independently traced rock boundaries, excluding native foliage and bark.'))
    write_json(folder/'inventory.json',inventory)
    old=OUT/'scenery-round-2/assets/croisement02-northwest-rock-outcrop'
    manifest=json.loads((old/'source-masks.json').read_text());manifest['mask_inventory']=str(folder/'inventory.json')
    assignment=next(a for a in manifest['projections']['exterior']['assignments'] if a.get('asset_group')==old.name)
    assignment['mask_indices']=[380];assignment.pop('exclude_mask_indices',None)
    receivers=['building-035','building-036','building-133']
    manifest['projections']['exterior']['occluder_constraints']=[dict(reviewed=True,source_node=node,receiver_nodes=receivers,mask_indices=[381],reason='Foreign coarse context cannot hide source-traced rock pixels; canopy, shrub and terrain artwork remain excluded.',review_evidence=str(folder/'ownership-review.json')) for node in ['ground',*(f'building-{i:03}' for i in range(150))] if node not in receivers]
    write_json(folder/'assignments.json',manifest)
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA');source.putalpha(Image.fromarray(domain.astype('uint8')*255))
    board=Image.new('RGBA',source.size,'#777777');board.alpha_composite(source);board.crop((0,0,250,245)).resize((750,735),Image.Resampling.NEAREST).convert('RGB').save(folder/'ownership-sheet.png')
    write_json(folder/'ownership-review.json',dict(status='candidate; manual source review required',domain_sha256=sha(folder/'domain-380.png'),sheet_sha256=sha(folder/'ownership-sheet.png'),source_outline=OUTLINE,small_rock_outline=SMALL,excluded_native_masks=[0,54,133],notes=['The dark upper rear rock face belongs to the rock domain; native53 also carries adjacent tree artwork that must not become rock texture.','Left and top frame boundaries truncate physical rocks and require inferred completion. The allocation stops at the visible rocky foot and reserves foreground shrub54.']))

if __name__=='__main__':main()
