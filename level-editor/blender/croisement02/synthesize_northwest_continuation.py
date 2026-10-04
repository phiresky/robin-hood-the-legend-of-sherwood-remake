"""Synthesize only scoped upper-cliff unknowns from an exact native donor."""
import json
from pathlib import Path
import subprocess
import sys

import numpy as np
from PIL import Image

sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json


def main():
    root=OUT/'texture-fill-round-2/croisement02-northwest-rock-outcrop/complete-preparation/experiment-native-cliff-v3'
    manifest=json.loads((root/'views.json').read_text())
    w,h=manifest['tile_size']
    for index in range(8):
        output=root/f'transport-output-{index}.png'
        if not output.exists():
            command=[str(Path.home()/'.cargo/bin/texture-synthesis'),
                '--sample-masks',str(root/f'transport-donor-{index}.png'),
                '--out-size',f'{w+128}x{h}','--threads','1','--seed',str(35+index),
                '--inpaint',str(root/f'transport-keep-{index}.png'),'--out',str(output),
                'generate',str(root/f'transport-input-{index}.png')]
            print('Synthesizing view',index,flush=True)
            subprocess.run(command,check=True)
        before=np.asarray(Image.open(root/f'transport-input-{index}.png').convert('RGB'))
        after=np.asarray(Image.open(output).convert('RGB'))
        keep=np.asarray(Image.open(root/f'transport-keep-{index}.png').convert('L'))>0
        if after.shape!=before.shape or not np.array_equal(before[keep],after[keep]):
            raise ValueError('Synthesis changed protected transport pixels')
    result=np.asarray(Image.open(root/'input.png').convert('RGBA')).copy()
    original=result.copy()
    unknown=np.asarray(Image.open(root/'mask.png').convert('RGBA'))[:,:,3]==0
    editable=np.asarray(Image.open(root/'upper-editable.png').convert('L'))>0
    donor=np.asarray(Image.open(root/'native-cliff-donor.png').convert('RGB'))
    palette=set(map(tuple,donor.reshape(-1,3).tolist()))
    base_root=Path(json.loads((root/'native-continuation-preparation.json').read_text())['parent'])
    base=np.asarray(Image.open(base_root/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png').convert('RGBA'))
    result[:]=base
    for index in range(8):
        tile=np.asarray(Image.open(root/f'transport-output-{index}.png').convert('RGB'))[:h,:w]
        box=(slice(index//4*h,(index//4+1)*h),slice(index%4*w,(index%4+1)*w))
        selected=editable[box]
        if any(tuple(pixel) not in palette for pixel in tile[selected].tolist()):
            raise ValueError('Continuation used foreign source pixels')
        result[box][:,:,:3][selected]=tile[selected]
    if not np.array_equal(result[~editable],base[~editable]):
        raise ValueError('Changed a pixel outside upper-cliff scope')
    if not np.array_equal(result[~unknown],original[~unknown]) or not np.array_equal(result[:,:,3],original[:,:,3]):
        raise ValueError('Changed native known RGB or physical image alpha')
    Image.fromarray(result).save(root/'native-generated-raw.png')
    Image.fromarray(result).save(root/'native-generated-preserved.png')
    write_json(root/'native-generation.json',dict(status='guards PASS; manual image review pending',
        provider='local texture-synthesis0.8.3; earlier API fill retained outside upper cliff',
        source_donor=str(root/'native-cliff-donor.png'),source_donor_sha256=sha(root/'native-cliff-donor.png'),
        target_mask_sha256=sha(root/'upper-editable.png'),source_rgb_preserved=True,
        alpha_preserved=True,all_other_fill_preserved=True,donor_palette_exact=True,
        transport=dict(size=[w+128,h],crop=[0,0,w,h],resize=False),
        generated_sha256=sha(root/'native-generated-preserved.png')))


if __name__=='__main__':main()
