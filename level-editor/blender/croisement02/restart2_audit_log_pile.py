"""Reopen the solid pile and check bank support, contacts and unchanged fallen logs."""
import json,hashlib,sys,math
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from restart2_build_log_pile import signature


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    dest=OUT/'restart2-state/log-triangular-pile-v5';fit=json.loads((dest/'fit.json').read_text());model=dest/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(OUT/'log-trap-state-candidate-v14/worker.blend'));old=signature([o for o in bpy.context.scene.objects if o.get('state_endpoint')=='applied']);bank=Path(fit['bank_model']);assert sha(bank)==fit['bank_model_sha256'];names=json.loads((bank.parent/'modified/views.json').read_text())['object_names'];bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update();source_deps=bpy.context.evaluated_depsgraph_get();frozen={name:bpy.data.objects[name].evaluated_get(source_deps).matrix_world.copy()for name in names};bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();fallen=signature([o for o in scene.objects if o.get('state_endpoint')=='applied']);assert fallen==old;logs=sorted([o for o in scene.objects if o.get('state_endpoint')=='covered'],key=lambda o:o.name);assert len(logs)==21;topology=[]
    for o in logs:
        bm=bmesh.new();bm.from_mesh(o.data);assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.free();topology.append(dict(name=o.name,closed_volume=volume))
    bank=Path(fit['bank_model']);assert sha(bank)==fit['bank_model_sha256'];names=json.loads((bank.parent/'modified/views.json').read_text())['object_names']
    with bpy.data.libraries.load(str(bank),link=False)as(src,dst):dst.objects=names
    for o in dst.objects:scene.collection.objects.link(o);o.parent=None;o.matrix_world=frozen[o.name]
    bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();verts=[];faces=[];transforms=[]
    for o in dst.objects:
        evaluated=o.evaluated_get(deps);matrix=evaluated.matrix_world.copy();mesh=evaluated.to_mesh();offset=len(verts);verts.extend(matrix@v.co for v in mesh.vertices);faces.extend(tuple(offset+i for i in p.vertices)for p in mesh.polygons);transforms.append(dict(name=o.name,matrix=[list(r)for r in matrix]));evaluated.to_mesh_clear()
    bank_max_z=max(v.z for v in verts);log_min_z=min((o.matrix_world@v.co).z for o in logs for v in o.data.vertices);assert log_min_z-bank_max_z>-.001
    bvh=BVHTree.FromPolygons(verts,faces);bank_rows=[]
    for row in fit['records']:
        if row['layer']!=0:continue
        a,b=Vector(row['start']),Vector(row['end']);clearances=[];touching=[];missing=0
        for t in np.linspace(0,1,101):
            p=a+(b-a)*float(t);p.z-=row['radius'];hit=bvh.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)),4000)[0];
            if hit is None:missing+=1;continue
            clearance=p.z-hit.z;clearances.append(clearance)
            if abs(clearance)<.002:touching.append(float(t))
        assert min(clearances)>-.001,(row['column'],min(clearances));assert touching and min(touching)<=.5<=max(touching),(row['column'],touching);bank_rows.append(dict(column=row['column'],samples=101,missing_bank_samples=missing,bank_contact_fraction=len(touching)/101,contact_parameter_span=[min(touching),max(touching)],center_inside_support_span=True,minimum_clearance=min(clearances),maximum_clearance=max(clearances)))
    u=np.array(fit['axis']);records=fit['records'];contacts=[];allgaps=[]
    for i,a in enumerate(records):
        aa,ab=np.array(a['start']),np.array(a['end']);ac=(aa+ab)/2;alo,ahi=aa@u,ab@u
        for j,b in enumerate(records[:i]):
            ba,bb=np.array(b['start']),np.array(b['end']);bc=(ba+bb)/2;delta=ac-bc;radial=delta-u*(delta@u);axial=max(0,ba@u-ahi,alo-bb@u);gap=float(math.hypot(np.linalg.norm(radial),axial)-a['radius']-b['radius']);allgaps.append(gap)
            if abs(gap)<.001:contacts.append(dict(first=i,second=j,circular_surface_clearance=gap))
    assert min(allgaps)>-.001
    assert sha(model)==digest
    report=dict(status='PASS reopened closed volumes, circular separation and flat bank support; visual and exact faceted contact review separate',model_sha256=digest,bank_model_sha256=fit['bank_model_sha256'],bank_evaluated_transforms=transforms,bank_support=bank_rows,global_bank_maximum_z=bank_max_z,global_log_minimum_z=log_min_z,global_separation_lower_bound=log_min_z-bank_max_z,topology=topology,minimum_circular_pair_clearance=min(allgaps),near_contacts=contacts,maximum_faceted_radial_deficit=fit['radius']*(1-math.cos(math.pi/24)),fallen_signature=fallen,fallen_unchanged=True,limitations=['Cylinder faceting can leave up to twice the radial deficit between ideal circular contacts.','Supporting-course count and hidden ends are inferred; this is not a native body identity or physics simulation proof.'])
    (dest/'reopened-support-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(len(logs),len(contacts),min(allgaps),[(r['column'],r['minimum_clearance'],r['maximum_clearance'])for r in bank_rows])
if __name__=='__main__':main()
