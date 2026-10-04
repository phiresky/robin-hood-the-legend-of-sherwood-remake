"""Align an already-rendered native-angle root view with its map artwork."""
import argparse,json,math,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json


def main(candidate):
    packet=json.loads((candidate/'roots-views.json').read_text());view=packet['views'][0]
    if abs(view['azimuth_degrees'])>1e-5:raise ValueError('Requires native azimuth')
    baseline=json.loads((OUT/'tree35-root-research/baseline-v2/views.json').read_text())['views'][0]
    if baseline['camera_matrix_world']!=view['camera_matrix_world'] or baseline['ortho_scale']!=view['ortho_scale']:raise ValueError('Root camera changed between comparisons')
    sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35));location=view['camera_location'];scale=view['ortho_scale'];camera_left=location[0]-scale/2;camera_top=-(location[1]*sin+location[2]*cos)-scale/2
    crop=[math.ceil(camera_left)+2,max(760,math.ceil(camera_top)+2),math.floor(camera_left+scale)-2,min(875,math.floor(camera_top+scale)-2)];w,h=crop[2]-crop[0],crop[3]-crop[1]
    frames=[]
    for path in [OUT/'tree35-root-research/baseline-v2/views/view-0-textured.png',candidate/'roots/view-0-textured.png']:
        im=Image.open(path).convert('RGBA');solid=np.asarray(Image.open(path.with_name(path.name.replace('textured','solid'))).convert('RGB'));im.putalpha(Image.fromarray((solid.max(axis=2)>4).astype('uint8')*255));pixels=im.width/scale;frames.append(im.transform((w,h),Image.Transform.AFFINE,(pixels,0,(crop[0]-camera_left)*pixels,0,pixels,(crop[1]-camera_top)*pixels),Image.Resampling.BILINEAR))
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(tuple(crop));sheet=Image.new('RGB',(w*5,h+14),(75,75,75));d=ImageDraw.Draw(sheet)
    panels=[source]
    for frame in frames:
        panel=Image.new('RGBA',frame.size,(100,100,100,255));panel.alpha_composite(frame);panels.append(panel)
    for frame in frames:
        panel=source.copy();panel.alpha_composite(frame);panels.append(panel)
    for i,(panel,title) in enumerate(zip(panels,['Source','Approved','New candidate','Approved overlay','Candidate overlay'])):sheet.paste(panel.convert('RGB'),(w*i,14));d.text((w*i+2,2),title,fill='white')
    sheet.resize((sheet.width*3,sheet.height*3),Image.Resampling.NEAREST).save(candidate/'native-root-comparison.png')
    write_json(candidate/'native-root-comparison.json',dict(model_sha256=sha(candidate/'model.blend'),comparison_sha256=sha(candidate/'native-root-comparison.png'),camera_manifest_sha256=sha(candidate/'roots-views.json'),source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),actual_view_sha256=sha(candidate/'roots/view-0-textured.png'),solid_view_sha256=sha(candidate/'roots/view-0-solid.png'),source_crop=crop,method='Existing actual-material root view0 has native35-degree camera orientation; image resampled to map pixel grid using frozen orthographic camera. Display alpha is derived from the paired opaque solid render against black; this is a visual alignment diagnostic, not an exact native silhouette audit.'))

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('candidate',type=Path);main(p.parse_args().candidate.resolve())
