"""Exact own-native gold leaf crop proposals; no synthesis or ownership changes."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;assert not out.exists();out.mkdir();pins={}
 def pin(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 inv=json.loads(pin(B/'source-inventory.json').read_text());frames=next(a for a in inv['animations']if a['index']==4)['frames'];row=frames[0];raw=np.array(Image.open(pin(Path(row['source']))).convert('RGBA'));assert pins[row['source']]==row['sha256'];expected=raw.copy();expected[np.all(raw[:,:,:3]==[0,251,0],axis=2)]=0;native=Image.open(pin(Path(row['image']))).convert('RGBA');assert np.array_equal(expected,np.array(native));x,y,w,h=row['bbox'];assert native.size==(w,h)
 positions=[(90,30),(150,30),(220,30),(65,70),(125,75),(195,75),(275,85),(45,125),(105,145),(245,145),(165,190),(300,125)]
 atlas=Image.new('RGBA',(16*len(positions),16));sheet=Image.new('RGB',(4*280,3*205),'#222222');draw=ImageDraw.Draw(sheet);records=[]
 for i,(u,v)in enumerate(positions):
  patch=native.crop((u,v,u+16,v+16));dest=out/f'native-gold-{i:02}.png';patch.save(dest);atlas.paste(patch,(16*i,0));ox=(i%4)*280;oy=(i//4)*205
  checker=Image.new('RGBA',(16,16));d=ImageDraw.Draw(checker)
  for py in range(16):
   for px in range(16):d.point((px,py),fill=(80,80,80,255)if(px//4+py//4)%2 else(150,150,150,255))
  checker=Image.alpha_composite(checker,patch);sheet.paste(checker.convert('RGB').resize((128,128),Image.Resampling.NEAREST),(ox+5,oy+45));black=Image.new('RGBA',(16,16),(0,0,0,255));black=Image.alpha_composite(black,patch);sheet.paste(black.convert('RGB').resize((128,128),Image.Resampling.NEAREST),(ox+143,oy+45));draw.text((ox+5,oy+5),f'{i:02} frame00 local {u},{v} 16x16',fill='white');draw.text((ox+5,oy+20),f'source {x+u},{y+v}; exact alpha',fill='white')
  records.append({'id':i,'phase':0,'local_xywh':[u,v,16,16],'map_xywh':[x+u,y+v,16,16],'actual_alpha_pixels':int((np.array(patch)[:,:,3]>0).sum()),'sha256':hashlib.sha256(dest.read_bytes()).hexdigest(),'role':'OWN_NATIVE_GOLD_LEAF_AND_FINE_TWIG_DONOR_PROPOSAL','not_green_ivy_donor':True})
 atlas.save(out/'exact-rgba-atlas.png');sheet.save(out/'donor-review-sheet.png');report={'status':'EXACT_NATIVE_DONORS_PENDING_VISUAL_REVIEW','pins':pins,'raw_to_normalized_full_rgba_exact':True,'patches':records,'atlas_layout':{'patch_size':[16,16],'columns':12,'rows':1},'limits':['Native alpha preserved exactly including holes. No background or gray stone is introduced.','Fine native dark twigs remain part of the artwork; these are not pure semantic leaf masks.','Use only for inferred rear gold crown if reviewed; never replace observed native source or mix into green ivy.','No synthesis, recoloring, model writes or source assignments.'],'output_pins':{p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in out.iterdir()if p.is_file()}};(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
