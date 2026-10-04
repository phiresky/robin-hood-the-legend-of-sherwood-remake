"""Bind net endpoint source pixels to current nearby wood without inventing anchors."""
import json,hashlib,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT,tree_workspace
from tree_geometry import RAY,SIN,COS
from log_trap_state_candidate import point

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    dest=OUT/'net-support-context-v1';dest.mkdir(exist_ok=False)
    source=OUT/'net-state-bindings-v1/manifest.json';assemblies=json.loads(source.read_text())['assemblies'];worker=tree_workspace(46)/'model.blend';before=sha(worker)
    bpy.ops.wm.open_mainfile(filepath=str(worker));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-46' and o.get('projection_component')!='crown'];vertices=[];faces=[]
    for o in objects:
        start=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(start+i for i in f.vertices)for f in o.data.polygons)
    bvh=BVHTree.FromPolygons(vertices,faces);records=[]
    for assembly in assemblies:
        for suffix in ['e','i']:
            patch=next(p for p in assembly['patches']if p['name'].endswith(suffix));frame=patch['states']['final']['frames'][0];path=OUT/'source-states'/frame['image'];rgba=np.array(Image.open(path).convert('RGBA'));alpha=(rgba[:,:,3]>0)&~np.all(rgba[:,:,:3]==[0,0,255],axis=2);x,y,w,h=frame['bbox'];hits=[];hitmask=np.zeros(alpha.shape,dtype=np.uint8)
            for yy,xx in zip(*np.nonzero(alpha)):
                ray=point(x+xx+.5,y+yy+.5,0);hit=bvh.ray_cast(ray+RAY*2000,-RAY,4000)
                if hit[0]is not None:hitmask[yy,xx]=255;hits.append(dict(native_pixel=[int(x+xx),int(y+yy)],world_point=list(hit[0])))
            name=f"{assembly['id']}-{suffix}";Image.fromarray(hitmask).save(dest/f'{name}-wood-ray-hits.png');records.append(dict(id=name,source_sha256=sha(path),bbox=frame['bbox'],native_opaque_pixels=int(alpha.sum()),wood_ray_hit_pixels=len(hits),wood_hit_z_range=[min(r['world_point'][2]for r in hits),max(r['world_point'][2]for r in hits)]if hits else None,hits=hits))
    assert sha(worker)==before
    result=dict(status='context evidence only; projection overlap is not a proven rope attachment',source_manifest_sha256=sha(source),wood_worker=str(worker),wood_worker_sha256=before,wood_objects=[o.name for o in objects],records=records,limitations=['Only current tree46 wood is ray-tested; crowns are excluded and no wood geometry changes.','A net pixel overlapping a wood ray does not place the net on that wood surface or establish front/back order.','Net03 has no native final-mask overlap and needs its own support geometry/source context.','Distinct e/i bag shapes and all final loop phases remain separate state requirements.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['id'],r['wood_ray_hit_pixels'],r['wood_hit_z_range'])for r in records])
if __name__=='__main__':main()
