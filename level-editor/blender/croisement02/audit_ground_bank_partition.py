"""Bind disjoint flat-ground and raised-bank observed source ownership."""
import argparse
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,bank_workspace
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('workspace',type=Path);args=parser.parse_args()
    worker=args.workspace.resolve();reference=worker/'reference';output=worker/'inspection'
    packet=json.loads((reference/'packet.json').read_text())
    bank=bank_workspace('croisement02-north-woodland-bank');bank_root=OUT/'terrain-bank-candidate'
    if sha(bank/'model.blend')!=packet['bank_model_sha256']:raise ValueError('Bank geometry changed')
    raster=bank_root/'integration/bank-ground-first-hit.npz'
    if sha(raster)!=packet['bank_first_hit_sha256']:raise ValueError('Bank/ground first-hit changed')
    evidence=json.loads((raster.parent/'evidence.json').read_text())
    if evidence['bank_model_sha256']!=sha(bank/'model.blend'):raise ValueError('First-hit evidence belongs to another bank')
    known=np.asarray(Image.open(reference/'ground-observed-domain.png'))>0
    reserved=np.asarray(Image.open(bank_root/'bank-source-domain.png'))>0
    depths=np.load(raster);names=list(depths['names']);owner=depths['owner']
    bank_hit=(owner>=0)&(owner!=names.index('ground'))
    domain_overlap=known&reserved;geometry_overlap=known&bank_hit
    if domain_overlap.any() or geometry_overlap.any():raise ValueError('Ground duplicates raised-bank source pixels')
    missing=reserved&~bank_hit
    source=np.asarray(Image.open(reference/'source.png').convert('RGB'));overlay=source.copy()
    overlay[known]=(source[known]*.55+np.array([0,210,210])*.45).astype('uint8')
    overlay[reserved]=(source[reserved]*.55+np.array([255,100,0])*.45).astype('uint8')
    overlay[missing]=[255,0,0]
    image=Image.fromarray(overlay);draw=ImageDraw.Draw(image)
    level_path=OUT/'baseline/Croisement02.rhp.json';level=json.loads(level_path.read_text())
    foot,=[r for r in level['elevation_lines'] if r['right_obstacle_index']==3 and r['left_obstacle_index']==65535]
    draw.line([tuple(foot['point_a']),tuple(foot['point_b'])],fill='white',width=2)
    image.save(output/'ground-bank-ownership-overlay.png')
    image.crop((900,0,1792,500)).resize((1338,750)).save(output/'northeast-ground-bank-ownership.png')
    write_json(output/'ground-bank-partition.json',dict(status='PASS',model_sha256=sha(worker/'model.blend'),bank_model_sha256=sha(bank/'model.blend'),bank_source_domain_sha256=sha(bank_root/'bank-source-domain.png'),ground_observed_domain_sha256=sha(reference/'ground-observed-domain.png'),first_hit_sha256=sha(raster),native_level_sha256=sha(level_path),observed_domain_overlap_pixels=int(domain_overlap.sum()),observed_ground_on_bank_geometry_pixels=int(geometry_overlap.sum()),bank_source_pixels_missing_geometry=int(missing.sum()),bank_misses_transferred_to_ground=int((missing&known).sum()),native_ramp3_ground_transition=foot,interpretation='White line marks native ramp3-to-ground transition. Orange is bank observed ownership; cyan is observed ground; red is known bank source without matching bank first hit, still withheld from ground. Dirt artwork below the transition remains on the native ground plane.',scope='Bank0–4 versus native flat ground. Separate scenery and full assembled-scene visibility remain separate checks.',images={name:sha(output/name) for name in ['ground-bank-ownership-overlay.png','northeast-ground-bank-ownership.png']}))


if __name__=='__main__':main()
