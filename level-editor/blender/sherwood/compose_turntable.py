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
parser.add_argument('--hq', action='store_true', help='High-resolution, 60 fps, 24-second slower turntable')
parser.add_argument('--split', action='store_true', help='Original baseline on left, refined scene on right')
parser.add_argument('--side-by-side', action='store_true', help='Two matched 960x1080 views in a 1920x1080 video')
parser.add_argument('--output-dir', type=Path, help='Alternate pass/output directory')
args = parser.parse_args()
if args.side_by_side and (not args.hq or args.split):
    parser.error('--side-by-side requires --hq and cannot be combined with --split')
comparison = args.split or args.side_by_side
if args.hq and args.fast:
    parser.error('--hq and --fast are mutually exclusive')
if args.split and not (args.fast or args.hq):
    parser.error('--split requires --fast or --hq and the matched original baseline passes')
if args.hq and not comparison:
    parser.error('--hq requires --split or --side-by-side to select the matching render layout')
OUT = Path(__file__).resolve().parents[3] / 'level-editor/work/sherwood-refinement' / ('turntable-side-by-side' if args.side_by_side else 'turntable-hq' if args.hq else 'turntable-fast' if args.fast else 'turntable')
if args.output_dir:
    OUT = args.output_dir.resolve()
W, H = (1920, 1080) if args.side_by_side else (1920, 1280) if args.hq else (960, 640) if args.fast else (1440, 1080)
SOURCE_W = W//2 if args.side_by_side else W
FPS = 60 if args.hq else 15 if args.fast else 25
FRAMES = list(range(5,1445)) if args.hq else [1 + round(i*25/15) for i in range(180)] if args.fast else list(range(1, 301))
BASELINE = OUT/'original' if args.hq else OUT.parent/'turntable-original'
TRANSITIONS = [(81, 'textured', 'solid'), (181, 'solid', 'wireframe'),
               (281, 'wireframe', 'textured')]
font = ImageFont.load_default(size=36 if args.side_by_side else 44 if args.hq else 22)
small = ImageFont.load_default(size=26 if args.side_by_side else 32 if args.hq else 16)
text_style = {'stroke_width':2,'stroke_fill':(20,26,31)} if args.side_by_side else {}
y, x = np.mgrid[0:H, 0:SOURCE_W]
radial = np.clip(1 - np.sqrt(((x-SOURCE_W/2)/(SOURCE_W*.75))**2 + ((y-H*.48)/(H*.85))**2), 0, 1)
background = Image.fromarray(np.stack([16+radial*13, 21+radial*15, 26+radial*18], axis=2).astype(np.uint8)).convert('RGBA')


def source_image(mode, frame, directory=OUT):
    path = directory / f'{mode}-{frame:04}.png'
    im = Image.open(path).convert('RGBA')
    if im.size != (SOURCE_W, H):
        raise ValueError(f'Wrong resolution: {path}: {im.size}')
    return Image.alpha_composite(background, im).convert('RGB')


def read(mode, frame):
    im = source_image(mode, frame)
    if args.side_by_side:
        before = source_image(mode, frame, BASELINE)
        pair = Image.new('RGB',(W,H))
        pair.paste(before,(0,0))
        pair.paste(im,(W//2,0))
        return pair
    if args.split:
        before = source_image(mode, frame, BASELINE)
        im.paste(before.crop((0, 0, W//2, H)), (0, 0))
    return im


boxes = []
clipped = 0
for output_frame, frame in enumerate(FRAMES, 1):
    timeline = frame*5/24 if args.hq else frame
    transition = next((t for t in TRANSITIONS if t[0] <= timeline < t[0]+20), None)
    if transition:
        start, old, new = transition
        progress = min(1., (timeline - start) / 19)
        if frame == FRAMES[-1]:
            progress = 1.0
        progress = progress * progress * (3 - 2 * progress)
        edge = round(progress * (W//2 if comparison else W))
        im = read(old, frame)
        replacement = read(new, frame)
        im.paste(replacement.crop((0, 0, edge, H)), (0, 0))
        if comparison:
            im.paste(replacement.crop((W//2, 0, W//2+edge, H)), (W//2, 0))
        label = new.capitalize() if progress >= .5 else old.capitalize()
        if 0 < edge < (W//2 if comparison else W):
            ImageDraw.Draw(im).line((edge, 0, edge, H), fill=(210, 220, 224), width=2)
            if comparison:
                ImageDraw.Draw(im).line((W//2+edge, 0, W//2+edge, H), fill=(210, 220, 224), width=2)
    else:
        mode = 'textured' if timeline < 81 or timeline >= 301 else 'solid' if timeline < 181 else 'wireframe'
        im = read(mode, frame)
        label = mode.capitalize()
    draw = ImageDraw.Draw(im)
    draw.text((35, 29), 'BEFORE / Original' if comparison else 'SHERWOOD', font=font, fill=(225, 231, 231), **text_style)
    draw.text((35, 78 if args.side_by_side else 88 if args.hq else 62), label, font=small, fill=(151, 174, 181), **text_style)
    if comparison:
        draw.line((W//2, 0, W//2, H), fill=(235, 237, 235), width=2)
        draw.text((W//2+35, 29), 'AFTER / Refined', font=font, fill=(225, 231, 231), **text_style)
        draw.text((W//2+35, 78 if args.side_by_side else 88 if args.hq else 62), label, font=small, fill=(151, 174, 181), **text_style)
    im.save(OUT / f'final-{output_frame:04}.png', compress_level=2 if args.fast or args.hq else 6)
    # Validate all source passes stay inside the image; transparent margin is expected.
    modes = transition[1:] if transition else [mode]
    for directory in [OUT, BASELINE] if comparison else [OUT]:
        for name in modes:
            bbox = Image.open(directory / f'{name}-{frame:04}.png').getchannel('A').getbbox()
            if bbox is None:
                raise RuntimeError(f'Empty render: {directory.name} {name} {frame}')
            if bbox[0] == 0 or bbox[1] == 0 or bbox[2] == SOURCE_W or bbox[3] == H:
                clipped += 1
                if not (args.fast or args.hq):
                    raise RuntimeError(f'Scene touches frame edge: {name} {frame}: {bbox}')
            boxes.append(bbox)

video = OUT / ('sherwood-before-after.mp4' if comparison else 'sherwood-turntable.mp4')
subprocess.run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'warning', '-framerate', str(FPS),
                '-i', str(OUT / 'final-%04d.png'), '-frames:v', str(len(FRAMES)), '-c:v', 'libx264',
                '-preset', 'veryfast' if args.fast else 'medium' if args.hq else 'slow', '-crf', '24' if args.fast else '18',
                '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(video)], check=True)
sheet = Image.new('RGB', (1440, 1080))
for index, frame in enumerate([1,241,433,673,913,1153] if args.hq else [1, 31, 55, 85, 115, 145] if args.fast else [1, 51, 91, 141, 191, 241]):
    im = Image.open(OUT / f'final-{frame:04}.png')
    im.thumbnail((480, 540))
    sheet.paste(im, ((index % 3)*480, (index // 3)*540 + (540-im.height)//2))
sheet.save(OUT / 'turntable-contact-sheet.jpg', quality=95)
Image.open(OUT / ('final-0241.png' if args.hq else 'final-0031.png' if args.fast else 'final-0051.png')).save(OUT / 'turntable-poster.jpg', quality=95)
report = {'frames': len(FRAMES), 'fps': FPS, 'seconds': len(FRAMES)/FPS, 'resolution': [W, H],
          'render_passes': len(boxes), 'clipped_render_passes': clipped, 'cropping_allowed': args.fast or args.hq,
          'center_split': args.split, 'side_by_side': args.side_by_side,
          'left': 'Untouched original baseline' if comparison else None,
          'minimum_border_pixels': min(min(a, b, SOURCE_W-c, H-d) for a, b, c, d in boxes),
          'video': str(video)}
(OUT / 'video-validation.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report))
