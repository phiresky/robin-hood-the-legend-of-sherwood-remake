"""Measure native phase motion for all remaining crown groups without editing assets."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '1')
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from forest_layout import CROWNS

BASE = Path(__file__).resolve().parents[2] / 'work/active/croisement02'
MANIFEST = BASE / 'animation-references/manifest.json'
OFFSETS = np.array([(dx, dy) for dy in range(-6, 7) for dx in range(-6, 7)])
RADIUS = 6
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()


def match(a, b, x, y):
    patch = a[y-RADIUS:y+RADIUS+1, x-RADIUS:x+RADIUS+1]
    windows = np.lib.stride_tricks.sliding_window_view(b, (13, 13), axis=(0, 1))
    candidates = windows[y+OFFSETS[:, 1]-RADIUS, x+OFFSETS[:, 0]-RADIUS]
    scores = np.mean((candidates-patch.transpose(2, 0, 1))**2, axis=(1, 2, 3))
    k = int(scores.argmin())
    dx, dy = map(int, OFFSETS[k])
    far = (np.abs(OFFSETS[:, 0]-dx)>1) | (np.abs(OFFSETS[:, 1]-dy)>1)
    return dx, dy, float(scores[k]), float(scores[84]), float(scores[far].min())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    manifest = json.loads(MANIFEST.read_text())
    summaries = []
    for index, seeds in CROWNS.items():
        if index == 1:
            continue
        animation = next(a for a in manifest['animations'] if a['index'] == index)
        left = min(f['bbox'][0] for f in animation['frames'])-24
        top = min(f['bbox'][1] for f in animation['frames'])-24
        right = max(f['bbox'][0]+f['bbox'][2] for f in animation['frames'])+24
        bottom = max(f['bbox'][1]+f['bbox'][3] for f in animation['frames'])+24
        frames = []
        pins = []
        for f in animation['frames']:
            a = np.asarray(Image.open(f['image']).convert('RGBA'))
            x,y,w,h = f['bbox']
            assert a.shape == (h,w,4)
            canvas = np.zeros((bottom-top, right-left, 4), np.uint8)
            canvas[y-top:y-top+h,x-left:x-left+w] = a
            frames.append(canvas)
            pins.append(dict(path=f['image'], sha256=sha(Path(f['image'])), bbox=f['bbox'], delay=f['delay']))
        features = [np.concatenate((a[:,:,:3]/255*(a[:,:,3:]/255),a[:,:,3:]/255),axis=2).astype('float32') for a in frames]
        # Distributed interior probes per inferred crown; exact ownership is not claimed.
        samples = []
        for mask,cx,cy in seeds:
            possible = []
            for y in range(24,bottom-top-24,16):
                for x in range(24,right-left-24,16):
                    # Native stippled transparency can alias an even-spaced grid.
                    # Require patch evidence, never a single selected pixel.
                    if np.count_nonzero(frames[0][y-RADIUS:y+RADIUS+1,x-RADIUS:x+RADIUS+1,3]) < 12:
                        continue
                    owner = min(seeds,key=lambda s:(x+left-s[1])**2+(y+top-s[2])**2)[0]
                    if owner == mask:
                        possible.append((x,y))
            # Spread probes across the owned region rather than clustering around its seed.
            chosen = []
            if possible:
                chosen.append(min(possible,key=lambda p:(p[0]+left-cx)**2+(p[1]+top-cy)**2))
                while len(chosen)<min(12,len(possible)):
                    chosen.append(max((p for p in possible if p not in chosen),key=lambda p:min((p[0]-q[0])**2+(p[1]-q[1])**2 for q in chosen)))
            samples.extend((mask,x,y) for x,y in chosen)
            assert chosen, f'No supported probes for crown {mask}'
        rows = []
        for phase in range(14):
            measured = []
            for mask,x,y in samples:
                dx,dy,cost,zero,second = match(features[0],features[phase],x,y)
                bx,by,_,_,_ = match(features[phase],features[0],x+dx,y+dy)
                accepted = abs(dx+bx)<=1 and abs(dy+by)<=1 and cost<=.12 and (cost==0 or second>=cost*1.08)
                measured.append(dict(crown_mask=mask,pixel=[x+left,y+top],delta=[dx,dy],cost=cost,zero_cost=zero,second_cost=second,roundtrip=[dx+bx,dy+by],accepted=accepted))
            rows.append(dict(phase=phase,first_tick=phase*4,samples=measured,accepted=sum(r['accepted'] for r in measured)))
        dest=args.output/f'animation-{index:02}'
        dest.mkdir()
        report=dict(status='PRIVATE_SPARSE_SOURCE_MOTION_NOT_GEOMETRY_APPROVAL',animation_index=index,crown_masks=[s[0] for s in seeds],source_manifest_sha256=sha(MANIFEST),recipe_sha256=sha(Path(__file__)),source_frames=pins,cycle_ticks=56,rows=rows,limitations=['Twelve sparse probes per inferred crown, not dense deformation coverage.','Crown ownership uses existing nearest seed hypotheses; touching crowns need source-mask review.','Only source-plane displacement is measured; depth and hidden motion remain inferred.','No canonical geometry, material or texture bytes changed.'])
        (dest/'motion.json').write_text(json.dumps(report,indent=2)+'\n')
        summary=dict(animation_index=index,crowns=len(seeds),probes=len(samples),accepted_by_phase=[r['accepted'] for r in rows],report_sha256=sha(dest/'motion.json'))
        summaries.append(summary)
        print(json.dumps(summary),flush=True)
    (args.output/'batch.json').write_text(json.dumps(dict(status='SEVEN_SOURCE_MOTION_BATCHES',groups=summaries),indent=2)+'\n')


if __name__=='__main__':
    main()
