"""Reopen each cargo/debris assembly and audit finite support on approved ground."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from scipy.spatial import ConvexHull

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release


def audit(worker):
    manifest=json.loads((worker/'manifest.json').read_text())
    model=worker/'worker.blend';assert sha(model)==manifest['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
    objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];rows={};bvhs={};samples=[]
    for obj in objects:
        bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges),obj.name
        assert bm.calc_volume(signed=True)>0,obj.name;bm.free()
        obj.data.calc_loop_triangles();verts=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);center=verts.mean(axis=0)
        tris=np.array([verts[list(t.vertices)]-center for t in obj.data.loop_triangles])
        signed=np.einsum('ij,ij->i',tris[:,0],np.cross(tris[:,1],tris[:,2]))/6;mass=signed.sum()
        centroid=center+(signed[:,None]*tris.sum(axis=1)/4).sum(axis=0)/mass
        rows[obj.name]=dict(mass=float(mass),centroid=centroid,contacts=verts[verts[:,2]<=.05],minimum_z=float(verts[:,2].min()))
        bvhs[obj.name]=BVHTree.FromPolygons([Vector(p) for p in verts],[p.vertices[:] for p in obj.data.polygons]);samples.extend(verts.tolist())
    edges={name:set() for name in rows}
    for i,a in enumerate(rows):
        for b in list(rows)[i+1:]:
            if bvhs[a].overlap(bvhs[b]):edges[a].add(b);edges[b].add(a)
    pending=set(rows);groups=[]
    while pending:
        component={next(iter(pending))}
        while True:
            expanded=component|set().union(*(edges[k] for k in component))
            if expanded==component:break
            component=expanded
        pending-=component;groups.append(component)
    supports=[]
    for names in groups:
        mass=sum(rows[n]['mass'] for n in names)
        center=sum(rows[n]['centroid']*rows[n]['mass'] for n in names)/mass
        contacts=np.concatenate([rows[n]['contacts'] for n in names])
        if len(contacts)<3:
            supports.append(dict(components=sorted(names),status='HOLD no finite support polygon',contacts=len(contacts)));continue
        xy=np.unique(contacts[:,:2],axis=0);hull=ConvexHull(xy)
        margin=-(hull.equations[:,:2]@center[:2]+hull.equations[:,2])
        supports.append(dict(components=sorted(names),status='PASS' if min(margin)>=-1e-5 else 'HOLD COM outside support',
            center_of_mass=center.tolist(),minimum_hull_margin=float(min(margin)),support_hull=xy[hull.vertices].tolist()))
    ground=json.loads((OUT/'restart2-textures/approved6-ground-scene-v1/assembly.json').read_text())['ground']
    ground_model=Path(ground['model']);assert sha(ground_model)==ground['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(ground_model));bpy.context.view_layer.update();vertices=[];faces=[]
    for obj in bpy.context.scene.objects:
        if obj.type!='MESH' or obj.hide_render:continue
        points=[obj.matrix_world@v.co for v in obj.data.vertices]
        if not points or max(abs(p.z) for p in points)>1e-3:continue
        start=len(vertices);vertices.extend(points);faces.extend(tuple(start+i for i in p.vertices) for p in obj.data.polygons)
    assert vertices,'No approved flat ground receiver'
    bvh=BVHTree.FromPolygons(vertices,faces);misses=0;clearance=[]
    for p in samples:
        hit=bvh.ray_cast(Vector((p[0],p[1],1000)),Vector((0,0,-1)),2000)
        if hit[0] is None:misses+=1
        else:clearance.append(p[2]-hit[0].z)
    write_json(worker/'reopened-support-audit.json',dict(model_sha256=manifest['model_sha256'],ground_model_sha256=ground['model_sha256'],
        status='PASS' if not misses and min(clearance)>=-.001 and all(r['status']=='PASS' for r in supports) else 'HOLD',
        sampled_vertices=len(samples),receiver_misses=misses,minimum_receiver_clearance=min(clearance),assemblies=supports,
        surface_contact_graph={k:sorted(v) for k,v in edges.items()},
        limitations=['Uniform-density static support; no friction, rolling or fracture simulation.',
                    'Appearance, native silhouette and source-role review remain separate.']))


def main():
    acquire()
    try:
        for name in ['barrel-v1','loose-wood-v1']:audit(OUT/'restart3-south-cart'/name)
    finally:release()


if __name__=='__main__':main()
