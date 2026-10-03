"""Replace the artificial circular canopy cut with an overlapping irregular edge."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import crown_geometry
from bark_materials import fill
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import validate,modified
from audit_candidates import audit
from render_tree import render_workspace


def main():
    w=OUT/'forest-v4-round-1/assets/croisement02-tree-25';record=json.loads((w/'inspection/refinement.json').read_text())
    if record.get('source_packet'):raise ValueError('Tree 25 revision already exists; inspect before another change')
    row=next(r for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==25)
    packet=json.loads(Path(row['packet']).read_text());x,y,width,height=packet['native_bbox'];rgba=np.asarray(Image.open(Path(row['packet']).parent/'complete-source.png')).copy()
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text());native_row=next(r for r in native['masks'] if r['index']==packet['native_mask']);native_alpha=np.asarray(Image.open(OUT/'baseline/masks'/native_row['png']).convert('L'))>0
    yy,xx=np.mgrid[:height,:width];dx=xx+x-135;dy=yy+y-703;distance=np.hypot(dx,dy);theta=np.arctan2(dy,dx)
    radius=float(distance[rgba[:,:,3]>127].max())
    # The neighboring crown is continuous artwork, not a circular cut line.
    # Expand that shared upper-right overlap and use irregular small lobes.
    upper_right=np.maximum(0,(dx-dy)/np.maximum(1,distance)/np.sqrt(2))**3
    edge=radius+28*upper_right+7*np.sin(11*theta)+4*np.sin(29*theta)+2*np.sin(67*theta)
    alpha=native_alpha&(distance<=edge);rgba[:,:,3]=alpha*255
    destination=w/'inspection/crown-source-revision-1';destination.mkdir(exist_ok=True);Image.fromarray(rgba).save(destination/'complete-source.png');Image.fromarray((alpha*255).astype('uint8')).save(destination/'coverage.png')
    ay,ax=np.nonzero(alpha);packet['bbox']=[x+int(ax.min()),y+int(ay.min()),int(np.ptp(ax))+1,int(np.ptp(ay))+1]
    packet['source_pixels']=int(alpha.sum());packet['lobes']=[dict(image=str(destination/'complete-source.png'))];packet['coverage_provenance']='Native canopy alpha intersected with an irregular overlapping inferred crown boundary; expanded upper-right overlap. No individual tree boundary exists in the shared native canopy mask.'
    write_json(destination/'partition.json',packet)
    cfg=json.loads((w/'workspace.json').read_text());path=Path(cfg['source_mask_manifest']);m=json.loads((w/'mask-reference/assignments.json').read_text())
    assignment=next(a for a in m['projections']['exterior']['assignments'] if a.get('source_node')=='building-120' and a.get('projection_component')=='crown')
    assignment['mask_indices']=[packet['native_mask']]
    assignment['review_note']='Shared native canopy is authoritative; the separate revised physical alpha limits this inferred tree crown.'
    write_json(path,m)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(w);write_json(path,m)
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name];crown=next(o for o in objects if o.get('projection_component')=='crown')
    record['crown']=crown_geometry(crown,packet,row['ground_y'],True);crown['foliage_backfaces_version']='v5';modified(w);record['bark']=fill(w,objects,25);validate(w);bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'))
    record['source_packet']=str(destination/'partition.json');record['model_sha256']=sha(w/'model.blend');record['limitations'].append('Circular upper-right cutoff replaced with a larger irregular overlap; exact individual crown ownership remains inferred.')
    write_json(w/'inspection/refinement.json',record);audit(w);render_workspace(w,256,release_slot=False);release()

if __name__=='__main__':main()
