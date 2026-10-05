"""Reconcile both castle cover sprites without collapsing their source states."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
DEST=OUT/'restart2/hall-cover-source-combinations-v1'
if DEST.exists():
    raise FileExistsError(DEST)
layers=json.loads((OUT/'source-states-complete/layers.json').read_text())
native_dir=OUT/'geometry-pass-01/native-state-source-v1'
native=json.loads((native_dir/'manifest.json').read_text())
inventory=json.loads((OUT/'baseline/masks/manifest.json').read_text())
mask_lookup={(r['layer'],r['layer_index']):r['index'] for r in inventory['masks']}
covered=Image.open(OUT/'source-states-complete/covered.png').convert('RGBA')
revealed=Image.open(OUT/'source-states-complete/revealed.png').convert('RGBA')
rasters,records={},{}
for patch_id in ('patch-001','patch-002'):
    record=next(r for r in native['records'] if r['id']==patch_id)
    frame=next(r for r in record['rows'] if r['action']=='PatchInitial')['frames'][0]
    path=native_dir/frame['image']
    if hashlib.sha256(path.read_bytes()).hexdigest()!=frame['sha256']:
        raise ValueError('Native cover frame changed')
    raster=Image.new('RGBA',covered.size)
    raster.alpha_composite(Image.open(path).convert('RGBA'),tuple(frame['bbox'][:2]))
    rasters[patch_id]=raster
    patch=next(p for p in layers['patches'] if p['id']==patch_id)
    records[patch_id]={'frame_sha256':frame['sha256'],'bbox':frame['bbox'],
                      'removed_masks_when_applied':[mask_lookup[m['layer'],m['index']] for m in patch['state']['old_masks']],
                      'removed_obstacles_when_applied':patch['state']['old_sight_obstacles'],
                      'door_indices':patch['state']['door_indices']}
removed_masks={m for record in records.values() for m in record['removed_masks_when_applied']}
if removed_masks.intersection((647,650)):
    raise ValueError('A retained wall or roof mask is removed by these covers')
roof_mask=next(m for m in inventory['masks'] if m['index']==650)
if roof_mask['obstacle_indices']!=[799,800]:
    raise ValueError('Retained roof mask ownership changed')
alphas={key:np.asarray(value.getchannel('A'))>0 for key,value in rasters.items()}
union=alphas['patch-001']|alphas['patch-002']
overlap=alphas['patch-001']&alphas['patch-002']
for patch in layers['patches']:
    if patch['id'] in rasters:
        continue
    graphic=patch.get('graphic')
    if not graphic:
        continue
    mask=Image.new('L',covered.size)
    mask.paste(Image.open(OUT/'source-states-complete'/graphic['alpha']).convert('L'),tuple(graphic['bbox'][:2]))
    if np.any((np.asarray(mask)>0)&union):
        raise ValueError(f"Additional cover overlaps the hall pair: {patch['id']}")
base=np.asarray(covered).copy()
base[union]=np.asarray(revealed)[union]
base=Image.fromarray(base)
expected=np.asarray(covered)
ordered=base.copy()
for patch_id in ('patch-001','patch-002'):
    ordered.alpha_composite(rasters[patch_id])
if not np.array_equal(np.asarray(ordered),expected):
    raise ValueError('Ordered native covers do not reproduce the frozen covered source')
reverse=base.copy()
for patch_id in ('patch-002','patch-001'):
    reverse.alpha_composite(rasters[patch_id])
reverse_different=int(np.any(np.asarray(reverse)[:,:,:3]!=expected[:,:,:3],axis=2).sum())
DEST.mkdir(parents=True)
crop=(2500,240,3090,870)
tile_width,tile_height=crop[2]-crop[0],crop[3]-crop[1]
sheet=Image.new('RGB',(tile_width*2,(tile_height+28)*2),'#101820')
states=[]
for index,(first,second) in enumerate(((True,True),(False,True),(True,False),(False,False))):
    selected={'patch-001':first,'patch-002':second}
    image=base.copy()
    for patch_id,present in selected.items():
        if present:
            image.alpha_composite(rasters[patch_id])
    name=f"patch001-{'initial' if first else 'applied'}_patch002-{'initial' if second else 'applied'}"
    path=DEST/f'{name}.png'
    image.save(path)
    x,y=(index%2)*tile_width,(index//2)*(tile_height+28)
    sheet.paste(image.crop(crop).convert('RGB'),(x,y+28))
    ImageDraw.Draw(sheet).text((x+5,y+8),f'Native source: 001 {"initial" if first else "applied"}, 002 {"initial" if second else "applied"}',fill='white')
    removed=[m for key,present in selected.items() if not present for m in records[key]['removed_masks_when_applied']]
    states.append({'image':path.name,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
                   'initial_cover_present':selected,'removed_native_masks':removed})
sheet.save(DEST/'native-source-combinations.png')
(DEST/'manifest.json').write_text(json.dumps({
    'status':'Exact source-composition evidence; no geometry or runtime-reachability approval',
    'source_projection':'Original native orthographic artwork; all four tiles use the same source crop',
    'crop':crop,'initial_composition_order':['patch-001','patch-002'],
    'layers_sha256':hashlib.sha256((OUT/'source-states-complete/layers.json').read_bytes()).hexdigest(),
    'mask_inventory_sha256':hashlib.sha256((OUT/'baseline/masks/manifest.json').read_bytes()).hexdigest(),
    'initial_matches_frozen_covered_exactly':True,
    'overlapping_opaque_pixels':int(overlap.sum()),'reverse_order_different_pixels':reverse_different,
    'native_records':records,'states':states,
    'retained_source_masks':{'650':'Retained roof silhouette links obstacles 799 and 800; not removed by either cover.',
                             '647':'Short front wall return; not removed by either cover.'},
    'limitations':['These are four source combinations, not a proof that every combination is reachable in every mission.','Neighboring cover geometry, triggers, native masks and animation playback still require implementation and review.']},indent=2)+'\n')
print(json.dumps({'overlap':int(overlap.sum()),'reverse_order_different_pixels':reverse_different,'states':len(states)}))
