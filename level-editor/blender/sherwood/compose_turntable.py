"""Composite matched Blender passes with moving swipes and encode the video.

Normal Python, Pillow, NumPy and ffmpeg. Blender RGBA frames remain available
for alternative edits. This edits render outputs, not source game artwork.
"""
import json
import argparse
from pathlib import Path
import subprocess
import numpy as np
from PIL import Image, ImageDraw, ImageFont

parser = argparse.ArgumentParser()
parser.add_argument('--fast', action='store_true', help='New fuller-crown 3:2 preview, sampled at 15 fps')
parser.add_argument('--split', action='store_true', help='Original baseline on left, refined scene on right')
args = parser.parse_args()
if args.split and not args.fast:
    parser.error('--split requires --fast and the matched original baseline passes')
OUT = Path(__file__).resolve().parents[3] / 'level-editor/work/sherwood-refinement' / ('turntable-fast' if args.fast else 'turntable')
W, H = (960, 640) if args.fast else (1440, 1080)
FPS = 15 if args.fast else 25
FRAMES = [1 + round(i*25/15) for i in range(180)] if args.fast else list(range(1, 301))
TRANSITIONS = [(81, 'textured', 'solid'), (181, 'solid', 'wireframe'),
               (281, 'wireframe', 'textured')]
font = ImageFont.load_default(size=22)
small = ImageFont.load_default(size=16)
y, x = np.mgrid[0:H, 0:W]
radial = np.clip(1 - np.sqrt(((x-W/2)/(W*.75))**2 + ((y-H*.48)/(H*.85))**2), 0, 1)
background = Image.fromarray(np.stack([16+radial*13, 21+radial*15, 26+radial*18], axis=2).astype(np.uint8)).convert('RGBA')


def source_image(mode, frame, directory=OUT):
    path = directory / f'{mode}-{frame:04}.png'
    im = Image.open(path).convert('RGBA')
    if im.size != (W, H):
        raise ValueError(f'Wrong resolution: {path}: {im.size}')
    return Image.alpha_composite(background, im).convert('RGB')


def read(mode, frame):
    im = source_image(mode, frame)
    if args.split:
        before = source_image(mode, frame, OUT.parent/'turntable-original')
        im.paste(before.crop((0, 0, W//2, H)), (0, 0))
    return im


boxes = []
clipped = 0
for output_frame, frame in enumerate(FRAMES, 1):
    transition = next((t for t in TRANSITIONS if t[0] <= frame <= t[0]+19), None)
    if transition:
        start, old, new = transition
        progress = (frame - start) / 19
        if frame == FRAMES[-1]:
            progress = 1.0
        progress = progress * progress * (3 - 2 * progress)
        edge = round(progress * (W//2 if args.split else W))
        im = read(old, frame)
        replacement = read(new, frame)
        im.paste(replacement.crop((0, 0, edge, H)), (0, 0))
        if args.split:
            im.paste(replacement.crop((W//2, 0, W//2+edge, H)), (W//2, 0))
        label = new.capitalize() if progress >= .5 else old.capitalize()
        if 0 < edge < (W//2 if args.split else W):
            ImageDraw.Draw(im).line((edge, 0, edge, H), fill=(210, 220, 224), width=2)
            if args.split:
                ImageDraw.Draw(im).line((W//2+edge, 0, W//2+edge, H), fill=(210, 220, 224), width=2)
    else:
        mode = 'textured' if frame < 81 else 'solid' if frame < 181 else 'wireframe'
        im = read(mode, frame)
        label = mode.capitalize()
    draw = ImageDraw.Draw(im)
    draw.text((35, 29), 'BEFORE / Original' if args.split else 'SHERWOOD', font=font, fill=(225, 231, 231))
    draw.text((35, 62), label, font=small, fill=(151, 174, 181))
    if args.split:
        draw.line((W//2, 0, W//2, H), fill=(235, 237, 235), width=2)
        draw.text((W//2+35, 29), 'AFTER / Refined', font=font, fill=(225, 231, 231))
        draw.text((W//2+35, 62), label, font=small, fill=(151, 174, 181))
    im.save(OUT / f'final-{output_frame:04}.png', compress_level=2 if args.fast else 6)
    # Validate all source passes stay inside the image; transparent margin is expected.
    modes = transition[1:] if transition else [mode]
    for name in modes:
        bbox = Image.open(OUT / f'{name}-{frame:04}.png').getchannel('A').getbbox()
        if bbox is None:
            raise RuntimeError(f'Empty render: {name} {frame}')
        if bbox[0] == 0 or bbox[1] == 0 or bbox[2] == W or bbox[3] == H:
            clipped += 1
            if not args.fast:
                raise RuntimeError(f'Scene touches frame edge: {name} {frame}: {bbox}')
        boxes.append(bbox)

video = OUT / ('sherwood-before-after.mp4' if args.split else 'sherwood-turntable.mp4')
subprocess.run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'warning', '-framerate', str(FPS),
                '-i', str(OUT / 'final-%04d.png'), '-frames:v', str(len(FRAMES)), '-c:v', 'libx264',
                '-preset', 'veryfast' if args.fast else 'slow', '-crf', '24' if args.fast else '18',
                '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(video)], check=True)
sheet = Image.new('RGB', (1440, 1080))
for index, frame in enumerate([1, 31, 55, 85, 115, 145] if args.fast else [1, 51, 91, 141, 191, 241]):
    im = Image.open(OUT / f'final-{frame:04}.png')
    im.thumbnail((480, 540))
    sheet.paste(im, ((index % 3)*480, (index // 3)*540 + (540-im.height)//2))
sheet.save(OUT / 'turntable-contact-sheet.jpg', quality=95)
Image.open(OUT / ('final-0031.png' if args.fast else 'final-0051.png')).save(OUT / 'turntable-poster.jpg', quality=95)
report = {'frames': len(FRAMES), 'fps': FPS, 'seconds': len(FRAMES)/FPS, 'resolution': [W, H],
          'render_passes': len(boxes), 'clipped_render_passes': clipped, 'cropping_allowed': args.fast,
          'center_split': args.split, 'left': 'Untouched original baseline' if args.split else None,
          'minimum_border_pixels': min(min(a, b, W-c, H-d) for a, b, c, d in boxes),
          'video': str(video)}
(OUT / 'video-validation.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report))
