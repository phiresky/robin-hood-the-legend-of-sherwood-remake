"""Bind native artwork and physical all-pose evidence with explicit planted-foot limits."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def main(version=9):
 base=OUT/f'restart2-fence/shrub57-sign-bend-v{version}';proof=base/'joint-proof';dest=base/'native-physical-comparison';dest.mkdir(exist_ok=False)
 report=json.loads((proof/'report.json').read_text());assert sha(base/'model.blend')==report['model_sha256']
 source=OUT/'baseline/covered.png';art=Image.open(source).convert('RGBA');box=(27,222,123,318)
 targets=OUT/'state-target-evidence/manifest.json';frames=next(p for p in json.loads(targets.read_text())['profiles'] if p['id']=='TG_Panel-12')['rows'][0]['frames']
 orderpath=OUT/'state-sign-candidate/native-order-reference-v3/manifest.json';order=next(r for r in json.loads(orderpath.read_text())['records'] if r['target_index']==7)
 animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];foot={(76,285),(73,286),(75,286),(76,286)};records=[];sheet=Image.new('RGB',(2304,2496),(60,60,60));close=Image.new('RGB',(1536,1536),(60,60,60))
 for phase in range(32):
  native=art.crop(box);f=frames[phase];sprite=Image.open(f['image']).convert('RGBA');native.alpha_composite(sprite,(75+int(f['offset'][0])-box[0],286+int(f['offset'][1])-box[1]))
  for a in order['overlapping_animations']:
   assert a['after_sign'];af=next(r for r in animations if r['index']==a['index'])['frames'][0];x,y,w,h=af['bbox'];native.alpha_composite(Image.open(af['image']).convert('RGBA'),(x-box[0],y-box[1]))
  native=native.resize((288,288),Image.Resampling.NEAREST);actual=Image.open(proof/f'pose-{phase:02}-actual.png').convert('RGBA');physical=Image.new('RGBA',(288,288),(60,60,60,255));physical.alpha_composite(actual)
  alone=np.array(Image.open(proof/f'pose-{phase:02}-alone.png'))[1::3,1::3,0]>127;joint=np.array(Image.open(proof/f'pose-{phase:02}-joint.png'))[1::3,1::3,0]>127
  yy,xx=np.nonzero(alone&~joint);pixels=list(zip((xx+27).tolist(),(yy+222).tolist()));assert set(pixels)<=foot,pixels
  tile=Image.new('RGB',(576,312),(60,60,60));tile.paste(native,(0,0));tile.paste(physical,(288,0));ImageDraw.Draw(tile).text((3,291),f'Pose{phase}: native art | physical context; foot pixels {len(pixels)}',fill='white');sheet.paste(tile,(phase%4*576,phase//4*312))
  crop=(39*3,52*3,57*3,70*3);pair=Image.new('RGB',(384,192));pair.paste(native.crop(crop).resize((192,192),Image.Resampling.NEAREST),(0,0));pair.paste(physical.crop(crop).resize((192,192),Image.Resampling.NEAREST),(192,0));close.paste(pair,(phase%4*384,phase//4*192))
  records.append(dict(phase=phase,residual_pixels=[list(p) for p in pixels],raw_source_sprite_sha256=sha(Path(f['image'])),physical_render_sha256=sha(proof/f'pose-{phase:02}-actual.png')))
 sheet.save(dest/'all32-native-physical.png');close.save(dest/'all32-foot-closeups.png')
 write_json(dest/'report.json',dict(status='Complete target7 phase review with bounded planted-post contact caveat',model_sha256=report['model_sha256'],physical_proof_sha256=sha(proof/'report.json'),native_art_sha256=sha(source),source_targets_sha256=sha(targets),native_order_sha256=sha(orderpath),exact_depth_diagnostic_sha256=sha(OUT/'restart2-fence/sign7-residual-depth-v2/report.json'),records=records,total_residual_samples=sum(len(r['residual_pixels']) for r in records),phase26_noncontact_residual_resolved=True,limitations=['Four possible native pixel centers at planted-post/bank contact have no positive exterior volume interval behind the complete post, within0.001 ray unit numerical precision.','Physical context contains listed neighbors only; neutral gray areas are absent background, not inferred source appearance.','Native-camera scene presentation and actual physical oblique depth remain different contracts.','No user approval, canonical selector update or publication implied.']))
 print(dest)
if __name__=='__main__':main()
