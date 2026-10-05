"""Attribute reserved trap underlay candidates without claiming foreign scenery pixels."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
DEST=OUT/'restart2-state/trap-underlay-domain-audit-v1'
def paste(full,local,x,y):
 h,w=local.shape;x=int(x);y=int(y);full[max(0,y):min(1152,y+h),max(0,x):min(1792,x+w)]|=local[max(0,-y):min(h,1152-y),max(0,-x):min(w,1792-x)]
def main():
 if DEST.exists():raise FileExistsError(DEST)
 DEST.mkdir();inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks'];static=np.zeros((1152,1792),bool);initial=np.zeros_like(static);body=np.zeros_like(static);background=np.zeros_like(static);sources=[]
 for row in inventory:
  if not row.get('png')or row['index']>=142:continue
  path=Path(row['png']);mask=np.array(Image.open(path).convert('L'))>0;paste(initial if row['index']in[136,137]else static,mask,*row['box_top_left'])
 for family in ['log-trap','rock-trap']:
  manifest=OUT/'state-target-evidence'/family/'manifest.json';data=json.loads(manifest.read_text())
  for part in data['parts']:
   for frame in part['frames']:
    source=Path(frame['image']);assert sha(source)==frame['image_sha256'];rgba=np.array(Image.open(source).convert('RGBA'));paste(body,rgba[:,:,3]>0,*(int(np.floor(p+o))for p,o in zip(part['position'],frame['offset'])));sources.append({'family':family,'sha256':sha(source),'source':str(source)})
 ledger=json.loads((OUT/'restart2-state/receiver-rebind-v2/report.json').read_text())
 for row in ledger['frames']:
  raw=np.array(Image.open(row['source']).convert('RGBA'));paste(background,raw[:,:,3]>0,*row['bbox'][:2])
 packet=json.loads((OUT/'ground-texture-preparation/packet.json').read_text());authored=np.zeros_like(static);authored_sources=[]
 for row in packet['authored_scenery_reservations']:
  if row['asset']=='croisement02-north-woodland-bank':continue
  path=Path(row['path']);mask=np.array(Image.open(path).convert('L'))>0
  if mask.shape==(1152,1792):authored|=mask
  else:
   source=next((r for r in inventory if r['index']==row['mask']),None)
   if source:paste(authored,mask,*source['box_top_left'])
   else:raise ValueError('Missing authored mask placement')
  authored_sources.append(row)
 static|=authored| (np.array(Image.open(OUT/'ground-texture-preparation/animated-first-frame-exclusion.png'))>0)
 atlas_path=OUT/'restart3-fence-receiver/terminal-v3/base-atlas.png';atlas=np.array(Image.open(atlas_path).convert('RGBA'));gray=np.all(atlas[:,:,:3]==127,axis=2);trap=initial|body|background;eligible=trap&~static;unknown=eligible&gray;observed_patch=background&unknown;body_only=unknown&~background
 for name,mask in [('trap-union',trap),('foreign-static',static),('eligible-trap',eligible),('gray-candidate',unknown),('gray-native-patch-covered',observed_patch),('gray-body-only',body_only)]:Image.fromarray(mask.astype('uint8')*255).save(DEST/(name+'.png'))
 crop=(400,390,1010,680);image=atlas[:,:,:3].copy();image[gray&static]=[170,70,200];image[unknown]=[0,230,230];image[observed_patch]=[255,170,0];Image.fromarray(image).crop(crop).resize((1220,580),Image.Resampling.NEAREST).save(DEST/'attribution.png');write_json(DEST/'report.json',{'status':'Domain proposal only; actual receiver binding and material packet pending','atlas_sha256':sha(atlas_path),'counts':{'trap_union':int(trap.sum()),'foreign_overlap':int((trap&static).sum()),'eligible_trap':int(eligible.sum()),'gray_candidate':int(unknown.sum()),'gray_native_patch_union':int(observed_patch.sum()),'gray_body_only':int(body_only.sum())},'diagnostic_colors':{'purple':'Gray base atlas reserved for other static/animated scenery; never editable by this lane','cyan':'Gray trap-owned body-only footprint candidate','orange':'Gray trap footprint also touched by one or more native background frames; dynamic source files remain exact'},'body_sources':sources,'foreign_authored_sources':authored_sources,'limits':['Base-atlas gray is an appearance diagnostic, not automatic terrain ownership.','Need current physical ground/bank first-hit for every candidate ray before any fill.','All dynamic source RGBA files remain immutable; an inferred underlay never replaces their state-specific appearance.','Foreign scenery reservations excluded even where moving trap sprites overlap them.']})
if __name__=='__main__':main()
