"""One bounded, source-pinned contact site; never saves scene or model data."""
from pathlib import Path
import sys,json,math
import bpy
from mathutils import Vector,Matrix
from PIL import Image
P=Path(__file__).resolve().parent;sys.path[:0]=[str(P),str(P.parents[1]/'refinement')]
from mound_contact_budget import digest,check,source_name,MIB,png_bound
from render_slots import acquire,release
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from restart4_stump_final_contact import frame

def main():
 tag,destination,permission=sys.argv[sys.argv.index('--')+1:];out=Path(destination);total=Path(permission).parent/'contacts-v3';site_root=total/tag;assert out.is_relative_to(site_root);authority=json.loads(Path(permission).read_text());assert tag in authority['allowed_sites'];assert authority['limits']==dict(site_mib=8,total_mib=160,file_mib=4,free_floor_gib=10,address_space_gib=12,rss_gib=8)
 worker=OUT/'restart15-hiding-mounds/all-placements-v1';model=worker/'model.blend';validation=json.loads((worker/'validation.json').read_text());assert digest(model)==validation['model_sha256']==authority['model_sha256'];row=next(r for r in validation['records']if r['tag']==tag);check(site_root,total)
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;bpy.context.preferences.filepaths.use_auto_save_temporary_files=False;scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.use_persistent_data=False;scene.use_nodes=False;scene.render.use_sequencer=False;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.image_settings.color_depth='8';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral contact inspection');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world
 with bpy.data.libraries.load(str(model),link=False)as(src,dst):dst.objects=list(row['objects'])
 own=[]
 for expected,obj in zip(row['objects'],dst.objects):
  assert obj is not None and obj.type=='MESH'and obj.parent is None;assert max(abs(obj.matrix_basis[i][j]-(1 if i==j else 0))for i in range(4)for j in range(4))<1e-8;scene.collection.objects.link(obj);obj.matrix_world=Matrix.Identity(4);obj.hide_render=False;own.append(obj)
 assert len(own)==67
 pins=[];receivers=[]
 def append(path,expected,source_rows):
  check(site_root,total);assert digest(path)==expected
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):
   resolved=tuple(source_name(r['name'],src.objects)for r in source_rows);assert len(set(resolved))==len(resolved);dst.objects=list(resolved)
  records=[]
  for rec,name,obj in zip(source_rows,resolved,dst.objects):
   assert obj is not None and obj.type=='MESH';scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=Matrix(rec['matrix_world']);obj.hide_render=False;receivers.append(obj);records.append(dict(source_name=name,name=obj.name,matrix_world=rec['matrix_world'],vertices=len(obj.data.vertices),polygons=len(obj.data.polygons)))
  pins.append(dict(path=str(path),sha256=expected,objects=records))
 for pin in validation['substitutions']:
  if pin['group']in ['GROUND','croisement02-north-woodland-bank']:append(Path(pin['model']),pin['sha256'],pin['objects'])
 static_pin=OUT/'restart2-textures/batch10-linked-static-v1/source-pins.json';wall=[r for r in json.loads(static_pin.read_text())['receivers'].values()if r['asset_group']=='croisement02-southeast-stone-wall-and-gate'and not r['hide_render']]
 for source in sorted({r['model']for r in wall}):
  selected=[r for r in wall if r['model']==source];append(Path(source),selected[0]['model_sha256'],[dict(name=r['object_name'],matrix_world=r['matrix_world'])for r in selected])
 bpy.context.view_layer.update()
 for pin in pins:
  for rec in pin['objects']:assert max(abs(bpy.data.objects[rec['name']].matrix_world[i][j]-rec['matrix_world'][i][j])for i in range(4)for j in range(4))<1e-6
 images=[]
 for name,direction in [('native',RAY),('side',Vector((COS,0,SIN))),('low-side',Vector((math.cos(math.radians(12)),0,math.sin(math.radians(12)))) )]:
  assert png_bound(480,480,4)<4*MIB;check(site_root,total,4*MIB);camera=frame(scene,own,direction,480,1.5);target=out/f'{name}.png';assert not target.exists();scene.render.filepath=str(target);bpy.ops.render.render(write_still=True);assert target.stat().st_size<4*MIB;images.append(dict(view=name,path=target.name,sha256=digest(target),camera_matrix=[list(v)for v in camera.matrix_world],ortho_scale=camera.data.ortho_scale));check(site_root,total)
 assert png_bound(1440,480,3)<4*MIB;check(site_root,total,4*MIB);canvas=Image.new('RGB',(1440,480),(35,35,35))
 for i,entry in enumerate(images):
  im=Image.open(out/entry['path']).convert('RGBA');canvas.paste(im,(i*480,0),im)
 canvas.save(out/'contact-three.png');check(site_root,total);assert digest(model)==authority['model_sha256'];assert all(digest(Path(p['path']))==p['sha256']for p in pins)
 report=dict(status='RENDERED_REVIEW_PENDING',site=tag,aliases=row['aliases'],model_sha256=authority['model_sha256'],validation_sha256=digest(worker/'validation.json'),static_pins_sha256=digest(static_pin),source_inputs=pins,images=images,sheet_sha256=digest(out/'contact-three.png'),limits=authority['limits'],selected_clumps=67,receiver_count=len(receivers),scope='Only exact ground/bank/southeast-wall receivers; original camera first. Neutral inspection world. No canopy or source-order claim. No model or scene writes.');check(site_root,total,128*1024);(out/'render-receipt.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
