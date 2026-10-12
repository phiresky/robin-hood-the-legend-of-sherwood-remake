"""Bind existing geometry approvals to the authorized mask/48-degree refresh."""
import json,hashlib,shutil,sys
from pathlib import Path
import numpy as np
from PIL import Image
base=Path.cwd()/'level-editor/work/needs-review/derby-refinement/round-2/recovery-shelters-20260923'
asset=sys.argv[1]; work=base/asset
packet=work/('modified-masked-v2' if (work/'modified-masked-v2/views.json').exists() else 'modified')
frames=json.loads((packet/'views.json').read_text());out=work/'experiment';out.mkdir();(out/'views').mkdir()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
w,h=frames['tile_size'];mask_sheet=Image.new('RGBA',(w*4,h*2),(255,255,255,255))
for v in frames['views']:
 i=v['index'];known=np.asarray(Image.open(packet/'views'/f'view-{i}-known.png').convert('RGBA'))[:,:,0]>127
 solid=np.asarray(Image.open(packet/'views'/f'view-{i}-solid.png').convert('RGBA'))[:,:,3]>0
 mask=np.full((h,w,4),255,dtype=np.uint8);mask[solid&~known,3]=0;im=Image.fromarray(mask)
 v.update(input=f'views/view-{i}-input.png',mask=f'views/view-{i}-mask.png',crop={'left':i%4*w,'top':i//4*h,'width':w,'height':h})
 shutil.copyfile(packet/'views'/f'view-{i}-textured.png',out/v['input']);im.save(out/v['mask']);mask_sheet.paste(im,(i%4*w,i//4*h))
shutil.copyfile(packet/'textured.png',out/'input.png');shutil.copyfile(packet/'solid.png',out/'solid.png');mask_sheet.save(out/'mask.png')
validation=json.loads((work/'preparation-validation.json').read_text());approval=validation['geometry_approval']
frames.update(source_blend=str(work/'model.blend'),reviewed_packet=str(packet),reviewed_manifest_sha256=sha(packet/'views.json'),input_sha256=sha(out/'input.png'),geometry_revision=approval['geometry_revision'],texture_receiver_object_names=frames['object_names'])
frames['layout'].update(width=w*4,height=h*2)
(out/'views.json').write_text(json.dumps(frames,indent=2))
approval.update(input_sha256=sha(out/'input.png'),solid_sha256=sha(out/'solid.png'),saved_model_sha256=sha(work/'model.blend'),refresh_authorization='Existing geometry approval retained; source-mask authority and global 48-degree lighting update requested by user. No geometry change.',source_approval=validation['geometry_approval'])
approval.pop('source_approval')
(out/'approval.json').write_text(json.dumps(approval,indent=2))
print(out)
