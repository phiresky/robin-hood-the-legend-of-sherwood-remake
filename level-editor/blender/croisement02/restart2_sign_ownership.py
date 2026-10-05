"""Separate native dark board texels from inferred timber in grazing sign poses."""
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from fit_native_sign import OUT, FACES, geometry, projected, COS, SIN
from evidence_io import sha, write_json


def main():
    base = OUT / 'state-sign-candidate'
    source = base / 'phase-appearance-v1'
    evidence = json.loads((source / 'evidence.json').read_text())
    assert sha(source / 'model.blend') == evidence['model_sha256']
    fit = json.loads((base / 'fit-v6/fit.json').read_text())
    p = list(fit['parameters'].values())
    board = geometry(p)[0]
    frames = next(r for r in fit['profile']['rows'] if r['action_id'] == 0)['frames']
    dest = OUT / 'restart2-fence/sign-dark-ownership-v1'
    dest.mkdir(exist_ok=False)
    sheet = Image.new('RGB', (1280, 440*2), (70, 70, 70))
    records = []
    for row_index, (phase, face) in enumerate([(8, 4), (24, 2)]):
        image_path = source / f'pose-{phase:02}-board-face-{face}.png'
        mask_path = image_path.with_name(image_path.stem + '-ownership.png')
        donor_path = base / f'native-fill-v1/board-face-{face}.png'
        rgb = np.asarray(Image.open(image_path).convert('RGB'))
        known = np.asarray(Image.open(mask_path).convert('L')) > 0
        donor = np.asarray(Image.open(donor_path).convert('RGB'))
        assert np.array_equal(rgb[~known], donor[~known])
        dark = rgb.max(2) < 25
        vertices = board[list(FACES[face])]
        normal = np.cross(vertices[1]-vertices[0], vertices[2]-vertices[0])
        normal /= np.linalg.norm(normal)
        if np.dot(normal, vertices[0]-board.mean(0)) < 0:
            normal = -normal
        angle = math.radians(p[7]-phase*11.25)
        direction = np.array([-math.sin(angle)*COS, -math.cos(angle)*COS, SIN])
        cosine = float(normal @ direction)
        record = dict(phase=phase, face=face, total_texels=known.size,
                      directly_observed_texels=int(known.sum()),
                      directly_observed_near_black_texels=int((dark & known).sum()),
                      inferred_texels=int((~known).sum()), inferred_near_black_texels=int((dark & ~known).sum()),
                      changed_inferred_texels=0, normal_camera_cosine=cosine,
                      angle_from_face_normal_degrees=math.degrees(math.acos(cosine)),
                      source_rgba_sha256=frames[phase]['image_sha256'], image_sha256=sha(image_path),
                      ownership_sha256=sha(mask_path), inferred_donor_sha256=sha(donor_path))
        records.append(record)
        native = Image.new('RGBA', (64, 72))
        f = frames[phase]
        native.alpha_composite(Image.open(f['image']).convert('RGBA'), (32+int(f['offset'][0]), 55+int(f['offset'][1])))
        native = native.resize((256, 288), Image.Resampling.NEAREST)
        outline = [(float(x)*4, float(y)*4) for x, y in projected(vertices, p, phase)]
        ImageDraw.Draw(native).line(outline+[outline[0]], fill=(255, 80, 180, 255), width=2)
        ownership = np.zeros((*known.shape, 3), np.uint8)
        ownership[known] = [40, 210, 100]
        ownership[~known] = [230, 150, 40]
        for image, x in [(native, 0), (Image.fromarray(rgb).resize((384,384), Image.Resampling.NEAREST),256),
                         (Image.fromarray(ownership).resize((384,384),Image.Resampling.NEAREST),640),
                         (Image.fromarray(donor).resize((256,256),Image.Resampling.NEAREST),1024)]:
            sheet.paste(image, (x, row_index*440), image.getchannel('A') if image.mode == 'RGBA' else None)
        draw = ImageDraw.Draw(sheet)
        draw.text((3,row_index*440+386), f'Pose{phase}, board face{face}: native projected outline / actual UV / green observed, orange inferred / prior donor', fill='white')
        draw.text((3,row_index*440+407), f'Observed{known.sum()}/{known.size}; observed dark{(dark&known).sum()}; hidden changed0; face/camera cosine{cosine:.3f}', fill='white')
    sheet.save(dest / 'native-face-ownership.png')
    write_json(dest / 'report.json', dict(
        model_sha256=evidence['model_sha256'], appearance_evidence_sha256=sha(source / 'evidence.json'),
        records=records, montage_sha256=sha(dest / 'native-face-ownership.png'),
        finding='Large dark patches lie on broad source-visible board faces in the fitted pose. Inferred face texels are unchanged from the prior own-timber donor.',
        limitations=['Visibility and texel-to-source correspondence follow the fitted solid sign hypothesis, not an independently supplied native3D model.',
                     'Per-pose source shading stays baked when inspected from a different camera. A view-dependent lighting reinterpretation would change representation and must preserve observed native RGBA.',
                     'This receipt does not grant independent appearance approval or resolve physical scene occlusion.']))


if __name__ == '__main__':
    main()
