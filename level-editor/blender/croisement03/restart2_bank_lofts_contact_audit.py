"""Independent saved shoulder contact distances and solid intersection volumes."""
import sys,json,hashlib,math,shutil,argparse
from pathlib import Path
import bpy,bmesh,numpy as np
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'))
from render_slots import acquire,release
B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-shoulder-lofts-saved-v1'
def main():
 global O
 ap=argparse.ArgumentParser();ap.add_argument('--folder',type=Path,default=O);args=ap.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else []);O=args.folder.resolve();assert not (O/'independent-contact-audit.json').exists()
 assert shutil.disk_usage(R).free>=10*1024**3
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3;mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3
  path=O/'worker.blend';digest=hashlib.sha256(path.read_bytes()).hexdigest();bpy.ops.wm.open_mainfile(filepath=str(path));interface=json.loads((O/'interface-construction.json').read_text());plan_path=Path(interface['geometry_plan']);assert hashlib.sha256(plan_path.read_bytes()).hexdigest()==interface['geometry_plan_sha256'];plan=json.loads(plan_path.read_text());S=math.sin(math.radians(35));C=math.cos(math.radians(35));points=np.array([[x,-y/S,z/C]for x,y,z in plan['guards']['interface52_54']]);distances={}
  for key in ['52','54']:
   obj=bpy.data.objects['Candidate bank '+key];v=np.array([list(obj.matrix_world@p.co)for p in obj.data.vertices]);distances[key]=[float(np.linalg.norm(v-p,axis=1).min())for p in points]
  intersections={}
  for a,b in [('52','53'),('52','54'),('53','54')]:
   original=bpy.data.objects['Candidate bank '+a];copy=original.copy();copy.data=original.data.copy();bpy.context.scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy;mod=copy.modifiers.new('Temporary solid contact test','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=bpy.data.objects['Candidate bank '+b];bpy.ops.object.modifier_apply(modifier=mod.name);bm=bmesh.new();bm.from_mesh(copy.data);intersections[a+'-'+b]={'absolute_volume':abs(bm.calc_volume()),'faces':len(bm.faces),'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges)};bm.free();bpy.data.objects.remove(copy,do_unlink=True)
  receipt={'model_sha256':digest,'shared_endpoint_nearest_vertex_error':distances,'intersection_solids':intersections,'null_material_slots':{o.name:[i for i,m in enumerate(o.data.materials)if m is None]for o in bpy.context.scene.objects if o.name.startswith('Candidate bank')},'limits':['Nearest-vertex test is independent of boundary BVH nearest-surface precision.','Contact volume and source constraints do not establish visual anatomy acceptance.']};(O/'independent-contact-audit.json').write_text(json.dumps(receipt,indent=2)+'\n');assert hashlib.sha256(path.read_bytes()).hexdigest()==digest;print(receipt)
 finally:release()
if __name__=='__main__':main()
