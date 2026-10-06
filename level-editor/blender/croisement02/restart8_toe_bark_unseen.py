"""Continue same-object generated bark onto bounded occupied atlas gaps."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from scipy.spatial import cKDTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart5_initial_net_unseen import occupancy
from restart8_bake_toe_bark import pixels,read,h
from bake_texture_candidate import snapshot
from render_slots import acquire,release
O=Path.cwd()/'level-editor/work/croisement02-refinement';B=O/'restart8-toe-bark-fill-v1'
def main(n):
 assert shutil.disk_usage(O).free>25*1024**3
 D=B/f'tree-{n}-fill-v1/shader-restored-v1';out=D.parent/('unseen-complete-v3' if len(sys.argv)>sys.argv.index('--')+2 else 'unseen-complete-v2');limit=float(sys.argv[sys.argv.index('--')+2]) if len(sys.argv)>sys.argv.index('--')+2 else 8;out.mkdir(exist_ok=False);proof=read(D/'preservation.json');assert h(D/'worker.blend')==proof['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.context.scene;names=set(proof['active_receivers']);before=snapshot(scene,names);original={im.name:hbytes(im)for im in bpy.data.images if im.packed_file};rows=[];changed=set()
 for row in read(D/'generated-atlas.json')['objects']:
  ob=scene.objects[row['object']];im=bpy.data.images[ob.name+' / reviewed bark'];support=bpy.data.images[ob.name+' / generation support'];a=pixels(im).copy();s=pixels(support).copy();hh,ww=a.shape[:2];occupied,world=occupancy(ob,'Reviewed bark generated',ww,hh);known=s[:,:,0]>.5;donors=occupied&known;target=occupied&~known;yy,xx=np.nonzero(target);dy,dx=np.nonzero(donors);dist,idx=cKDTree(world[donors]).query(world[target]);valid=dist<=limit;ty,tx=yy[valid],xx[valid];b=a.copy();b[ty,tx,:3]=a[dy[idx[valid]],dx[idx[valid]],:3];s2=s.copy();s2[ty,tx,:3]=1;assert np.array_equal(b[~target],a[~target]);assert np.array_equal(b[:,:,3],a[:,:,3]);im.pixels.foreach_set(b.ravel());im.update();im.pack();support.pixels.foreach_set(s2.ravel());support.update();support.pack();changed|={im.name,support.name};np.savez_compressed(out/(ob.name.rsplit(' ',1)[-1]+'-continuation.npz'),target_xy=np.column_stack((tx,ty)),donor_xy=np.column_stack((dx[idx[valid]],dy[idx[valid]])),world_distance=dist[valid]);rows.append(dict(object=ob.name,occupied=int(occupied.sum()),generated_donors=int(donors.sum()),unsupported_occupied=int(target.sum()),continued=int(valid.sum()),distance_limit=limit,maximum_distance=float(dist[valid].max(initial=0)),outside_bound=int((~valid).sum()),scope='Original-source protection shader remains final authority; occupied atlas continuation cannot override protected native pixels.'))
 assert snapshot(scene,names)==before
 assert all(hbytes(bpy.data.images[k])==v for k,v in original.items()if k not in changed)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);sha=h(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));assert snapshot(bpy.context.scene,names)==before
 assert all(hbytes(bpy.data.images[k])==v for k,v in original.items()if k not in changed)
 for f in ('close-views.json','full-views.json'):shutil.copyfile(D/f,out/f)
 result=dict(parent_model_sha256=proof['model_sha256'],model_sha256=sha,geometry_outside_appearance_exact=True,original_source_images_exact=True,only_occupied_generated_atlas_and_support_updated=True,objects=rows,limitations=['Outside bounded continuation retains previous fallback; final underside/source inspection required.','Padding unchanged. Original native/grazing source shaders still override generated layer.']);(out/'continuation.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
def hbytes(im):return hashlib.sha256(bytes(im.packed_file.data)).hexdigest()
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
