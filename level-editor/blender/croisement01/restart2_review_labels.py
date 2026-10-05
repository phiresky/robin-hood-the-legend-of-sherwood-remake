"""Add verified native-camera labels without replacing frozen review evidence."""
import hashlib
import json
import math
from pathlib import Path
from PIL import Image,ImageDraw


def labeled_sheet(worker,relative):
    worker=Path(worker);source=worker/relative
    packet=json.loads((worker/'modified/views.json').read_text())
    first=packet['views'][0];matrix=first['camera_matrix_world'];axis=[matrix[i][2] for i in range(3)]
    expected=[0,-math.cos(math.radians(35)),math.sin(math.radians(35))]
    if first['index']!=0 or max(abs(a-b) for a,b in zip(axis,expected))>1e-5:
        raise ValueError(f'First camera is not the original orthographic direction: {worker}')
    if 'terrain-contact' in relative or 'root-contact-detail' in relative:
        evidence=json.loads((source.parent/'evidence.json').read_text())
        if evidence['camera_indices'][0]!=0:raise ValueError('Contact sheet starts at a different camera')
    digest=hashlib.sha256(source.read_bytes()).hexdigest()
    folder=worker/'inspection/native-first-labels-v1';folder.mkdir(exist_ok=True)
    target=folder/(source.parent.name+'-'+source.stem+'-'+digest[:16]+'.png')
    if not target.exists():
        image=Image.open(source).convert('RGB');canvas=Image.new('RGB',(image.width,image.height+28),(28,28,28));canvas.paste(image,(0,28))
        ImageDraw.Draw(canvas).text((8,8),'TOP LEFT: ORIGINAL GAME CAMERA (orthographic, 35 deg elevation)',fill='white');canvas.save(target)
        target.with_suffix('.json').write_text(json.dumps(dict(source=str(source),source_sha256=digest,source_unchanged=True,camera_index=0,camera_axis=axis,projection='orthographic',label_only=True),indent=2)+'\n')
    return target
