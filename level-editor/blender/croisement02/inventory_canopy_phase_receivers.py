"""Inventory native canopy phase groups and explicit exceptional source domains."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
 dest=OUT/'canopy-phase-receiver-inventory-v1';dest.mkdir(exist_ok=True);assert not(dest/'manifest.json').exists()
 source=OUT/'animation-references/manifest.json';animations=json.loads(source.read_text())['animations'];groups={a['index']:dict(index=a['index'],profile=a['profile'],cycle_ticks=sum(f['delay']+1 for f in a['frames']),seconds=sum(f['delay']+1 for f in a['frames'])/25,assets=[],frames=[dict(path=f['image'],sha256=sha(Path(f['image'])),bbox=f['bbox'],delay=f['delay'],duration_ticks=f['delay']+1)for f in a['frames']])for a in animations if 'Arbre' in a['profile']}
 for path in sorted((OUT/'forest-v4-sources').glob('tree-*/partition.json')):
  p=json.loads(path.read_text());index=p['animation'];assert p['profile']==groups[index]['profile'];groups[index]['assets'].append(dict(asset='croisement02-'+path.parent.name,partition=str(path),partition_sha256=sha(path),source_bbox=p['bbox'],native_mask=p['native_mask'],physical_receiver_and_temporal_ownership_status='Pending current-model phase binding; source packet association alone is insufficient.'))
 assert len(groups)==8 and sum(len(g['assets'])for g in groups.values())==44
 fringe=OUT/'understory-candidates/north-fringe22-leaf-fill-v2/partition.json';f=json.loads(fringe.read_text());domain=Path(f['source_split_path']);mask=np.array(Image.open(domain).convert('L'))>0;h,w=mask.shape;x=y=0;assert (w,h)==(1792,1152)
 overlaps=[]
 for a in animations:
  if 'Arbre' not in a['profile']:continue
  rows=[]
  for frame in a['frames']:
   fx,fy,fw,fh=frame['bbox'];left,top=max(x,fx),max(y,fy);right,bottom=min(x+w,fx+fw),min(y+h,fy+fh)
   count=0
   if right>left and bottom>top:
    pixels=np.array(Image.open(frame['image']).convert('RGBA'))[:,:,3]>0
    count=int((mask[top-y:bottom-y,left-x:right-x]&pixels[top-fy:bottom-fy,left-fx:right-fx]).sum())
   rows.append(count)
  overlaps.append(dict(animation=a['index'],profile=a['profile'],phase_overlap_pixels=rows))
 special=[dict(asset='croisement02-tree-21',observed_canopy_pixels=0,status='Off-map inferred crown; no observed native canopy phase identity. Do not assign another tree animation as measured motion.'),dict(asset='croisement02-canopy-fringe-22',partition=str(fringe),partition_sha256=sha(fringe),observed_domain=str(domain),observed_domain_sha256=sha(domain),observed_domain_pixels=int(mask.sum()),native_animation_overlaps=overlaps,status='Separate authored fringe; overlap is only a diagnostic, not proof of phase ownership.')]
 result=dict(status='Source receiver inventory only; no models modified or animation completion claimed',animation_manifest_sha256=sha(source),native_clock_hz=25,groups=list(groups.values()),special_receivers=special,butterflies=[dict(index=a['index'],profile=a['profile'],frames=len(a['frames']),cycle_ticks=sum(f['delay']+1 for f in a['frames']),role='Native visual effect; retain presentation independently of tree geometry')for a in animations if 'papillon' in a['profile']],remaining=['Bind current reviewed model hashes and exact observed temporal domains.','Preserve static plus animated source composition and per-phase ownership.','Review physical appearance from all eight views and validate visible runtime playback.'])
 (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print([(g['index'],len(g['assets']))for g in groups.values()]);print(overlaps)
if __name__=='__main__':main()
