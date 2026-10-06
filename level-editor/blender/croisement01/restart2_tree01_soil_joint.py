"""Private local soil shoulder around the native tree root; no gameplay mutation."""
import argparse,json,math,shutil,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS
from render_slots import acquire
from evidence_io import sha
FRONT=lambda x:-782.9419+1.0176*(x-55)
R=OUT/'restart2';source=R/'tree01-v2/assets/croisement01-tree-01';dest=None;worker=None
def volume(name,xs,ts,lower,upper):
 verts=[];nx=len(xs);ny=len(ts)
 for surface in [lower,upper]:
  for x in xs:
   front=FRONT(x)
   for t in ts:verts.append((x,front+t,surface(x,t)))
 faces=[];n=nx*ny
 for i in range(nx-1):
  for j in range(ny-1):
   a=i*ny+j;b=a+ny;faces += [(a,a+1,b+1,b),(n+a,n+b,n+b+1,n+a+1)]
 boundary=list(range(ny))+[i*ny+ny-1 for i in range(1,nx)]+list(range((nx-1)*ny+ny-2,(nx-1)*ny-1,-1))+[i*ny for i in range(nx-2,0,-1)]
 for a,b in zip(boundary,boundary[1:]+boundary[:1]):faces.append((a,b,b+n,a+n))
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj);return obj

def main():
 global dest,worker,FRONT
 parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=2);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);dest=R/f'tree01-soil-joint-v{args.revision}';worker=dest/'assets/croisement01-tree-01'
 if shutil.disk_usage(R).free<35*1024**3:raise ValueError('Disk floor35GiB')
 dest.mkdir(exist_ok=False);worker.mkdir(parents=True);acquire();before=sha(source/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']];bank=next(o for o in collection.all_objects if o.get('source_node')=='building-008');tree=next(o for o in collection.all_objects if o.get('source_node')=='building-029')
 tree_positions=[list(v.co) for v in tree.data.vertices];bank_before=[list(bank.matrix_world@v.co) for v in bank.data.vertices]
 topface=max(bank.data.polygons,key=lambda f:sum((bank.matrix_world@bank.data.vertices[i].co).z for i in f.vertices)/len(f.vertices));top_points=[bank.matrix_world@bank.data.vertices[i].co for i in topface.vertices];assert len(top_points)==3
 if args.revision>=9:
  start=min(top_points,key=lambda p:p.x);end=max(top_points,key=lambda p:p.x);FRONT=lambda x:start.y+(end.y-start.y)*(x-start.x)/(end.x-start.x)
 points=top_points+[Vector((p.x,p.y,0)) for p in top_points];mesh=bpy.data.meshes.new('Closed archived bank shell');mesh.from_pydata([bank.matrix_world.inverted()@p for p in points],[],[(0,1,2),(5,4,3),(0,3,4,1),(1,4,5,2),(2,5,3,0)]);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free()
 for material in bank.data.materials:mesh.materials.append(material)
 bank.data=mesh
 (dest/'bank-shell-preparation.json').write_text(json.dumps(dict(method='Use exact native bank top triangle and zero bottom to close physical shell. Legacy separately snapped side faces had up to0.8unit gaps; their unchanged baseline is preserved.',top_points=[list(p) for p in top_points],gameplay_unchanged=True,appearance_approval=False),indent=2)+'\n')
 xs=[15+i*(2.5 if args.revision>=7 else 5) for i in range(33 if args.revision>=7 else 17)];top=173.001/COS
 if args.revision>=10:xs=[-13.594341278076172+i*(123.35006713867188+13.594341278076172)/56 for i in range(57)]
 def weight(x):return max(0,math.cos((x-55)/(70 if args.revision>=10 else 40)*math.pi/2)**2)
 def shoulder(x,t):
  if args.revision>=8:
   w=weight(x);lateral=min(1,max(0,abs(x-55)-18)/(52 if args.revision>=10 else 22));lateral=lateral*lateral*(3-2*lateral);height=159+(top-159)*lateral
   if t>=0:
    u=min(1,max(0,t-18)/62);smooth=u*u*(3-2*u);return height+(top-height)*smooth
   if args.revision>=10:
    u=min(1,max(0,-t-25)/95);smooth=u*u*(3-2*u);return height*(1-smooth)
   u=min(1,max(0,-t-25)/(120*max(.001,w)-25)) if 120*max(.001,w)>25 else min(1,-t/(120*max(.001,w)));smooth=u*u*(3-2*u);return height*(1-smooth)
  if args.revision>=7:
   w=weight(x)
   if t>=0:
    u=min(1,t/80);smooth=u*u*(3-2*u);return top-w*(top-155)*(1-smooth)
   u=min(1,-t/(120*max(.001,w)));smooth=u*u*(3-2*u);return (top-w*(top-155))*(1-smooth)
  original=top-weight(x)*(56*(1-max(0,t)/45) if t>=0 else 56+(-t)*2.4)
  blend=math.exp(-2*(((x-55)/25)**2+(t/25)**2)) if args.revision>=6 else 0
  return original*(1-blend)+159*blend
 cutter=volume('Local inferred soil shoulder cutter',xs,([i*5 for i in range(17)] if args.revision>=7 else [0,5,10,20,30,45]),shoulder,lambda x,t:top+200)
 bpy.context.view_layer.objects.active=bank;modifier=bank.modifiers.new('Local root shoulder recess','BOOLEAN');modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter;bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(cutter,do_unlink=True)
 # Add only the narrow shoulder in front of the old boundary, tapering its
 # outward reach to zero at the lateral limits. Soil depth remains inferred.
 verts=[];faces=[];nx=len(xs);ts=[0,.25,.5,.75,1]
 for lower in [True,False]:
  for x in xs:
   w=weight(x);front=FRONT(x)
   for f in ts:
    t=-(120 if args.revision>=7 else 22)*(1 if args.revision>=10 else max(.001,w))*f+(.5*(1-f) if args.revision>=9 else 0);verts.append((x,front+t,0 if lower else shoulder(x,t)))
 n=nx*5
 for i in range(nx-1):
  for j in range(4):
   a=i*5+j;b=a+5;faces +=[(a,b,b+1,a+1),(n+a,n+a+1,n+b+1,n+b)]
 boundary=list(range(5))+[i*5+4 for i in range(1,nx)]+list(range((nx-1)*5+3,(nx-1)*5-1,-1))+[i*5 for i in range(nx-2,0,-1)]
 for a,b in zip(boundary,boundary[1:]+boundary[:1]):faces.append((a,a+n,b+n,b))
 mesh=bpy.data.meshes.new('Local inferred soil shoulder');mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();soil=bpy.data.objects.new(mesh.name,mesh);collection.objects.link(soil)
 material=bpy.data.materials.new('Unapproved inferred soil');material.diffuse_color=(.32,.27,.17,1);mesh.materials.append(material)
 for face in soil.data.polygons:face.use_smooth=args.revision>=7
 soil['source_node']='tree01-local-soil-joint';soil['asset_group']='croisement01-tree01-local-soil';soil['part_name']='Inferred local bank shoulder';soil['review_scope']='Geometry only; observed source ownership separate from inferred soil depth'
 if args.revision>=9:
  bpy.context.view_layer.objects.active=bank;modifier=bank.modifiers.new('Continuous local soil union','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.object=soil;bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(soil,do_unlink=True)
  for face in bank.data.polygons:face.use_smooth=True
 assert tree_positions==[list(v.co) for v in tree.data.vertices]
 (worker/'modified').mkdir();(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
 assert sha(source/'model.blend')==before
 (dest/'construction.json').write_text(json.dumps(dict(status='Private diagnostic, not approved',baseline_sha256=before,model_sha256=sha(worker/'model.blend'),tree_vertices_unchanged=True,terrain_changed=(['building-008'] if args.revision>=9 else ['building-008','tree01-local-soil-joint']),gameplay_metadata_unchanged=True,observed='Native root silhouette and foreground ownership are fixed; local soil color only where unobscured own native artwork supports it.',inferred=('Hidden shoulder slope,120-unit tapered outward slope to baseline ground and80-unit rear recovery; not inferred from grayscale threshold as metric depth.' if args.revision>=7 else 'Hidden shoulder slope,22-unit outward extent and56-unit lowering near root; not inferred from grayscale threshold as metric depth.'),x_bounds=[min(xs),max(xs)],top_z=top,bank_vertices_before=bank_before),indent=2)+'\n')
 print(worker)
if __name__=='__main__':main()
