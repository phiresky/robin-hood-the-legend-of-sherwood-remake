"""Read the final saved hay atlas and topology under one shared render lease."""
import argparse,sys,json
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from audit_candidates import audit
from restart3_hay_texel_audit import main as texels

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--workspace',type=Path,required=True);parser.add_argument('--review-directory',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=args.workspace;h=sha(w/'model.blend');texels();audit(w)
 report=json.loads((args.review_directory/'texel-audit.json').read_text());rows=report['samples'];neutral=[r for r in rows if r['neighbor_error']>0 and len(set(r['atlas_rgb']))==1]
 if neutral:raise ValueError(('Neutral observed source remains',len(neutral)))
 old=json.loads((OUT/'restart3-hay/texel-audit-v8/texel-audit.json').read_text());protected={tuple(r['pixel']):r for r in old['samples']};changed=[]
 for row in rows:
  previous=protected[tuple(row['pixel'])]
  if row['atlas_rgb']!=previous['atlas_rgb']:
   if row['atlas_rgb']!=row['source_rgb'] or len(set(previous['atlas_rgb']))!=1:raise ValueError('Unexpected source color change')
   changed.append(row['pixel'])
 if len(changed)!=18 or report['geometry_gaps']!=old['geometry_gaps']:raise ValueError('Native domain or restoration count changed')
 source_dir=w/'inspection/source-coverage';original=Image.open(source_dir/'source.png').convert('RGBA');original.putalpha(Image.open(source_dir/'expected.png').convert('L'));actual=Image.open(source_dir/'render.png').convert('RGBA');width,height=actual.size;canvas=Image.new('RGBA',(width*2,height),(75,75,75,255));canvas.alpha_composite(original,(0,0));canvas.alpha_composite(actual,(width,0));canvas.resize((width*6,height*3),Image.Resampling.NEAREST).convert('RGB').save(args.review_directory/'source-comparison.png')
 write_json(args.review_directory/'final-verification.json',dict(model_sha256=h,source_samples=len(rows),geometry_gaps_unchanged=True,changed_native_samples=changed,other_first_hit_RGB_exact=True,neutral_native_samples=0,source_comparison_sha256=sha(args.review_directory/'source-comparison.png')))
 if sha(w/'model.blend')!=h:raise ValueError('Read-only verification changed model')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
