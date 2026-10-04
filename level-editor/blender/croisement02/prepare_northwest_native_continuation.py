"""Prepare a native-grain continuation confined to unknown upper-cliff pixels."""
import json
import os
from pathlib import Path
import shutil
import sys

import numpy as np
from PIL import Image

sys.path[:0] = [str(Path(__file__).parent), str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json


def main():
    root = OUT/'texture-fill-round-2/croisement02-northwest-rock-outcrop/complete-preparation'
    source = root/'experiment-cliff-retry-v2'
    scope = root/'upper-cliff-scope-v1'
    output = root/'experiment-native-cliff-v3'
    if output.exists():
        raise FileExistsError(output)
    preparation = json.loads((source/'preparation.json').read_text())
    scope_proof = json.loads((scope/'scope.json').read_text())
    if sha(source/'approved-model.blend') != scope_proof['approved_model_sha256'] or sha(source/'views.json') != scope_proof['views_sha256']:
        raise ValueError('Receiver scope does not match frozen packet')
    output.mkdir()
    for name, expected in preparation['files'].items():
        path = source/name
        if sha(path) != expected:
            raise ValueError('Frozen preparation changed: '+name)
        dest = output/name
        dest.parent.mkdir(exist_ok=True, parents=True)
        if name == 'approved-model.blend':
            os.link(path, dest)
        else:
            shutil.copy2(path, dest)
    shutil.copy2(source/'preparation.json',output/'preparation.json')
    input_rgba = np.asarray(Image.open(source/'input.png').convert('RGBA'))
    unknown = np.asarray(Image.open(source/'mask.png').convert('RGBA'))[:,:,3] == 0
    upper = np.asarray(Image.open(scope/'upper-atlas.png').convert('L')) > 0
    base_path = source/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png'
    base = np.asarray(Image.open(base_path).convert('RGBA'))
    if not np.array_equal(input_rgba[~unknown],base[~unknown]):
        raise ValueError('Prior fill changed protected input')
    editable = unknown & upper
    donor_box = [185,78,281,142]
    if not np.all((upper & ~unknown)[78:142,185:281]):
        raise ValueError('Cliff donor contains unknown or foreign receiver pixels')
    Image.fromarray(input_rgba[78:142,185:281,:3]).save(output/'native-cliff-donor.png')
    manifest = json.loads((source/'views.json').read_text())
    w,h = manifest['tile_size']
    for index in range(8):
        box = (slice(index//4*h,(index//4+1)*h),slice(index%4*w,(index%4+1)*w))
        pixels = base[box][:,:,:3]
        keep = (~editable[box]).astype('uint8')*255
        Image.fromarray(pixels).save(output/f'continuation-input-{index}.png')
        Image.fromarray(keep).save(output/f'continuation-keep-{index}.png')
        # The synthesis CLI's inpaint path cannot reliably combine multiple
        # examples. Put the exact native donor in a protected transport margin,
        # then crop that margin off without resizing the reviewed tile.
        transport = np.zeros((h,w+128,3),dtype='uint8')
        transport[:,:w] = pixels
        transport[:64,w+16:w+112] = input_rgba[78:142,185:281,:3]
        transport_keep = np.full((h,w+128),255,dtype='uint8')
        transport_keep[:,:w] = keep
        donor_mask = np.zeros((h,w+128),dtype='uint8')
        donor_mask[:64,w+16:w+112] = 255
        for label, image in [('input',transport),('keep',transport_keep),('donor',donor_mask)]:
            Image.fromarray(image).save(output/f'transport-{label}-{index}.png')
    Image.fromarray(editable.astype('uint8')*255).save(output/'upper-editable.png')
    write_json(output/'native-continuation-preparation.json',dict(
        status='private diagnostic input; no generation review', parent=str(source),
        parent_generated_sha256=sha(base_path), scope=str(scope/'scope.json'),
        scope_sha256=sha(scope/'scope.json'), donor_box=donor_box,
        donor_sha256=sha(output/'native-cliff-donor.png'), known_native_donor_pixels=6144,
        editable_upper_pixels=int(editable.sum()), all_other_pixels_protected=True,
        transport=dict(size=[w+128,h], crop=[0,0,w,h], donor_box=[w+16,0,w+112,64], resize=False),
        geometry_unchanged=True, model_sha256=sha(output/'approved-model.blend')))
    print(output)


if __name__=='__main__':
    main()
