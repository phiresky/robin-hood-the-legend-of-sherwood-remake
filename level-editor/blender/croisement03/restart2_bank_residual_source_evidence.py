"""Immutable full-resolution proposal evidence; no source acceptance or assignment."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';W=B/'restart2'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;assert not out.exists();out.mkdir();pins={}
 def pin(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 previous=json.loads(pin(W/'bank-residual-source-context-v2/receipt.json').read_text());src=Image.open(pin(B/'baseline/covered.png')).convert('RGBA');revealed=Image.open(pin(B/'baseline/revealed.png')).convert('RGBA');assert pins[str(B/'baseline/covered.png')]==previous['pins'][str(B/'baseline/covered.png')];classes=np.array(Image.open(pin(W/'bank-residual-source-context-v2/residual-review-classes.png')));original=classes==1;assert int(original.sum())==378;rows=[];total=np.zeros(original.shape,bool);scale=16
 for region in previous['patches']:
  if region['type']!='stone_window_review':continue
  points=region['polygon'];domain=Image.new('L',src.size);ImageDraw.Draw(domain).polygon([tuple(p)for p in points],fill=255);mask=(np.array(domain)>0)&original;assert int(mask.sum())==region['residual_pixels_in_patch'];assert not np.any(total&mask);total|=mask;yxs=np.argwhere(mask);x0,y0,x1,y1=region['source_bounds_xyxy'];box=(x0-7,y0-7,x1+7,y1+7);raw=src.crop(box);maskcrop=Image.fromarray(mask.astype('uint8')*255).crop(box);name=region['id'];raw.save(out/f'{name}-original-source.png');maskcrop.save(out/f'{name}-original-mask.png');raw.resize((raw.width*scale,raw.height*scale),Image.Resampling.NEAREST).save(out/f'{name}-unmarked-16x.png')
  width,height=raw.width*scale,raw.height*scale;left,top,right,bottom=55,65,20,35;panelwidth=width+left+right;sheet=Image.new('RGB',(panelwidth*2,height+top+bottom),'#292929');draw=ImageDraw.Draw(sheet)
  for col,(label,im)in enumerate([('UNMARKED original source',raw.convert('RGB')),('Original proposed pixels ONLY (white); NOT accepted',maskcrop.convert('RGB'))]):
   ox=col*panelwidth;sheet.paste(im.resize((width,height),Image.Resampling.NEAREST),(ox+left,top));draw.text((ox+5,5),name,fill='white');draw.text((ox+5,20),label,fill='white');draw.text((ox+5,35),f'Native crop XYXY{box}; scale16 nearest',fill='white')
   for x in range(box[0],box[2]):
    px=ox+left+(x-box[0])*scale+scale//2
    if x%5==0:draw.text((px-9,top-17),str(x),fill='white');draw.line((px,top-3,px,top-1),fill='white')
   for y in range(box[1],box[3]):
    py=top+(y-box[1])*scale+scale//2
    if y%5==0:draw.text((ox+left-29,py-5),str(y),fill='white');draw.line((ox+left-3,py,ox+left-1,py),fill='white')
  sheet.save(out/f'{name}-source-mask-16x.png');rows.append({'id':name,'source_bounds_xyxy':box,'original_polygon':points,'original_proposed_pixels':int(mask.sum()),'exact_proposed_pixels_xy':[[int(x),int(y)]for y,x in yxs],'source_crop_rgba_sha256':hashlib.sha256(np.array(raw).tobytes()).hexdigest(),'covered_revealed_crop_exact':bool(np.array_equal(np.array(raw),np.array(revealed.crop(box)))),'evidence':f'{name}-source-mask-16x.png','status':'HOLD_PENDING_PIXEL_REVIEW_NO_ASSIGNMENT'})
 assert np.array_equal(total,original)
 observations={
 'east-cap-shoulder-window':'HOLD114: mask crosses conspicuous dark diagonal feature from about[554,226] toward[563,231]. Adjacent pale stone is visible, but feature may be surface fissure or foreground twig; whole polygon not justified.',
 'central-upper-window-a':'HOLD42: gray/tan source visible, but left isolated mask pixels and right edge adjoin gold/green leaf fragments. Fragment-alpha exclusion does not independently establish remaining tiny sample roles.',
 'central-upper-window-b':'HOLD39: highly fragmented source window around dark/gold features; the lower checker-pattern mask islands cannot be accepted as coherent stone interior.',
 'central-upper-window-c':'HOLD53: central pale patch is plausible stone, but gold/olive features touch and enter its upper-left and right edges. Require a manually narrowed pixel list before projection.',
 'south-high-window':'HOLD88: strongest pale continuous stone interior, but mask left edge and bottom meet twig/leaf tips. Prioritize inspection of interior aroundx341..345,y339..344; this location is a review pointer, not a replacement accepted mask.',
 'south-right-small-window':'HOLD42; withdraw whole-polygon stone interpretation: most left/lower samples lie on a dark gap/twig mixture, while a small right ridge is lit. Neither dark color nor adjacency establishes stone. No full42projection.'}
 selfreview={'status':'HOLD_ALL378_AFTER_PERSONAL_UNMARKED16X_REVIEW','reviewed':[r['evidence']for r in rows],'original378_mask_unchanged':True,'accepted_pixels':0,'narrowed_assignment_mask_created':False,'findings':observations,'artifact_sha256':{path.name:hashlib.sha256(path.read_bytes()).hexdigest()for path in sorted(out.iterdir())if path.is_file()},'next':'Root reviews unmarked16x source and separate exact mask. Any future subset needs new exact pixel list, allphase foreground check and saved receiver proof; do not project original378 wholesale.'};(out/'self-review.json').write_text(json.dumps(selfreview,indent=2)+'\n')
 report={'status':'HOLD_ALL378_SOURCE_ROLE_REVIEW_ONLY','pins':pins,'original_proposal_pixels':378,'original_mask_exact':True,'scale':scale,'sampling':'Nearest neighbor; source panel has no tint, drawn mask, annotation or grid over source pixels. Axes are outside image. Separate binary mask panel shows exact existing proposal.','regions':rows,'limits':['Original378mask immutable; no polygon accepted or reassigned.','Source crop bounds and exact global pixel list supplied for per-pixel review.','Appearance review only; no geometry or saved receiver proof, no synthesis.']};(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['id'],r['original_proposed_pixels'],r['source_bounds_xyxy'])for r in rows])
if __name__=='__main__':main()
