"""Bind native canopy phases to existing 3D crown components, not extra billboards."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT,tree_workspace,reviewed_catalog
from forest_layout import CROWNS

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 destination=OUT/'state-candidate-v1/canopy-binding';destination.mkdir(exist_ok=False)
 native=OUT/'animation-references/manifest.json';animations=json.loads(native.read_text())['animations'];records=[]
 for index,seeds in CROWNS.items():
  animation=next(a for a in animations if a['index']==index);frames=animation['frames']
  left=min(f['bbox'][0] for f in frames);top=min(f['bbox'][1] for f in frames);right=max(f['bbox'][0]+f['bbox'][2] for f in frames);bottom=max(f['bbox'][1]+f['bbox'][3] for f in frames)
  arrays=[]
  for frame in frames:
   rgba=np.array(Image.open(frame['image']).convert('RGBA'));x,y,w,h=frame['bbox'];assert rgba.shape[:2]==(h,w)
   alpha=np.zeros((bottom-top,right-left),bool);alpha[y-top:y-top+h,x-left:x-left+w]=rgba[:,:,3]>127;arrays.append(alpha)
  union=np.logical_or.reduce(arrays);first=arrays[0];components=[]
  for mask,cx,cy in seeds:
   worker=tree_workspace(mask);views=json.loads((worker/'modified/views.json').read_text());crowns=[n for n in views['object_names'] if 'crown' in n.lower()];assert len(crowns)==1
   components.append(dict(asset_id=f'croisement02-tree-{mask:02}',wood_mask=mask,crown_object=crowns[0],worker=str(worker),model_sha256=sha(worker/'model.blend'),source_seed=[cx,cy],static_components=[n for n in views['object_names'] if n not in crowns],native_partition='inferred tree ownership within shared canopy; retained from reviewed model'))
  image=np.zeros((*first.shape,3),np.uint8);image[first]=[70,110,70];image[union&~first]=[255,0,255];Image.fromarray(image).save(destination/f'animation-{index:02}-phase-union.png')
  records.append(dict(animation_index=index,profile=animation['profile'],native_mask=128+index,components=components,
   source_union_bbox=[left,top,right-left,bottom-top],first_frame_alpha_pixels=int(first.sum()),union_alpha_pixels=int(union.sum()),new_pixels_in_later_phases=int((union&~first).sum()),
   phases=[dict(index=i,image=f['image'],image_sha256=sha(Path(f['image'])),delay=f['delay'],bbox=f['bbox'],alpha_new_vs_first=int((a&~first).sum()),alpha_removed_vs_first=int((first&~a).sum())) for i,(f,a) in enumerate(zip(frames,arrays))],
   representation='Animate only bound crown components. Do not instantiate the duplicate native canopy billboard on top of these meshes.',
   observed_animation='Use native phase pixels and measured source-plane motion on existing leaf clusters; preserve full depth and stationary wood.',
   hidden_animation='Any rear/side motion is inferred and must remain coherent with the measured front phase; source frames do not supply hidden RGB or depth.'))
 catalog=json.loads(reviewed_catalog().read_text());mapped={r['asset_id'] for a in records for r in a['components']};unmapped=[g['id'] for g in catalog['groups'] if 'wood_mask' in g and g['id'] not in mapped]
 result=dict(native_manifest_sha256=sha(native),animations=records,unmapped_crowns=unmapped,
  implementation_requirements=['Keep 7 non-canopy ambient native sequences as independent animated scenery.',
   'A shared phase clock belongs to each original canopy; all split components use its exact frame delays and offsets.',
   'Store identity/rest geometry as approved; per-phase displacement belongs to a separate reviewed animation layer.',
   'Sample source-phase RGB only for observed leaf material; retain accepted inferred rear textures.',
   'Later-phase alpha expands beyond frame0: cards and per-phase source domains must cover temporal union, with duplication/ownership audited against neighboring crowns.',
   'Use measured source-plane motion to deform existing 3D leaf clusters; keep depth coordinate and trunk/branch geometry fixed. Hidden motion remains explicitly inferred.',
   'Current static model exporter disables animations; publication needs an explicit supported morph/vertex-animation path and live editor playback validation.',
   'Native canopy billboard must be suppressed only when equivalent bound 3D animation is active, retaining native gameplay sorting/occlusion metadata.',
   'Unmapped boundary crown has no independent native sequence. Choose and label inferred coherent phase binding rather than fabricating observed animation.'],
  status='binding and temporal coverage evidence; actual 3D playback not yet implemented')
 (destination/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'canopies':len(records),'bound_crowns':len(mapped),'unmapped':unmapped,'new_phase_pixels':sum(a['new_pixels_in_later_phases'] for a in records)}))
if __name__=='__main__':main()
