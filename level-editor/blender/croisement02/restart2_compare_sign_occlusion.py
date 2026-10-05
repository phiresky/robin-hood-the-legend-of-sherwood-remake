"""Compare physical sign-body first hits against exact native overlay alpha."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json


def main():
    source = OUT / 'restart2-fence/sign-neighbors-v4'
    manifest = json.loads((source / 'manifest.json').read_text())
    animations = json.loads((OUT / 'animation-references/manifest.json').read_text())['animations']
    order = json.loads((OUT / 'state-sign-candidate/native-order-reference-v3/manifest.json').read_text())
    dest = OUT / 'restart2-fence/sign-occlusion-comparison-v1'
    dest.mkdir(exist_ok=False)
    records = []
    for row in manifest['results']:
        index = row['target_index']
        native = next(r for r in order['records'] if r['target_index'] == index)
        x, y = native['display_position']
        crop = (x-48, y-64, x+48, y+32)
        overlay = Image.new('RGBA', (1792,1152))
        overlays = []
        for a in native['overlapping_animations']:
            assert a['after_sign']
            frame = next(r for r in animations if r['index'] == a['index'])['frames'][0]
            fx, fy, fw, fh = frame['bbox']
            image = Image.open(frame['image']).convert('RGBA')
            assert image.size == (fw, fh)
            overlay.alpha_composite(image, (fx, fy))
            overlays.append(dict(index=a['index'], source_sha256=sha(Path(frame['image']))))
        alpha = np.asarray(overlay.crop(crop))[:, :, 3] > 127
        for phase in [0,8,16,24]:
            alone_path = source / f'target-{index}/pose-{phase:02}-body-alone.png'
            joint_path = source / f'target-{index}/pose-{phase:02}-body-first-hit.png'
            alone = np.asarray(Image.open(alone_path).convert('RGB'))[1::3,1::3,0] > 127
            joint = np.asarray(Image.open(joint_path).convert('RGB'))[1::3,1::3,0] > 127
            blocked = alone & ~joint
            expected_blocked = alone & alpha
            extra = blocked & ~alpha
            insufficient = alone & joint & alpha
            canvas = np.full((96,96,3), 70, np.uint8)
            canvas[alone] = [170,170,170]
            canvas[blocked & alpha] = [40,200,80]
            canvas[extra] = [255,60,40]
            canvas[insufficient] = [30,170,255]
            Image.fromarray(canvas).resize((384,384),Image.Resampling.NEAREST).save(dest / f'target-{index}-pose-{phase:02}.png')
            records.append(dict(target_index=index, sign_pose=phase, native_overlay_phase=0,
                                physical_body_pixels=int(alone.sum()), native_expected_blocked=int(expected_blocked.sum()),
                                physical_blocked=int(blocked.sum()), excess_occlusion=int(extra.sum()),
                                missing_occlusion=int(insufficient.sum()),
                                body_alone_sha256=sha(alone_path), physical_first_hit_sha256=sha(joint_path),
                                overlays=overlays))
    write_json(dest / 'report.json', dict(source_manifest_sha256=sha(source / 'manifest.json'),
               sign_model_sha256=manifest['sign_model_sha256'], records=records,
               legend='Gray physical sign body; green correct native overlay occlusion; red excess physical occlusion; blue missing native overlay occlusion.',
               limitations=['Comparison uses the same physical-body footprint for expected and actual occlusion, avoiding native sprite versus mesh silhouette denominator differences.',
                            'Only native overlay frame0 and four sign poses; moving actor and remaining canopy phase validation remain separate.',
                            'No source asset, anchor, material, or native overlay alpha is modified.']))
    print([(r['target_index'],r['sign_pose'],r['excess_occlusion'],r['missing_occlusion'])for r in records])


if __name__ == '__main__':
    main()
