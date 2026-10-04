"""Native mask 70 curved fallen branch construction; private review candidate."""
import sys
import json
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from prepare_props import ROOT, OUT, SIN, COS, prepare, modified, validate, acquire, release, sha

# Centers are measured in native source pixels; radii and elevation are inferred.
MAIN=[(1038,337,6,6),(1058,342,8,8),(1082,339,9,9),
      (1102,333,10,10),(1122,322,10,10),(1141,317,9,9),
      (1163,319,8,8),(1180,324,7,7),(1198,336,5,5),(1218,345,2,2)]
TWIG=[(1100,331,10,4),(1109,315,12,3.6),(1120,302,15,3),
      (1134,287,18,1.8)]


def tube(name,trace):
    centers=[Vector((x,-(y+z*COS)/SIN,z)) for x,y,z,r in trace]
    vertices=[];faces=[];n=16
    for i,center in enumerate(centers):
        tangent=(centers[min(i+1,len(centers)-1)]-centers[max(0,i-1)]).normalized()
        side=tangent.cross(Vector((0,0,1))).normalized();up=side.cross(tangent).normalized()
        for j in range(n):
            angle=2*math.pi*j/n
            vertices.append(center+trace[i][3]*(side*math.cos(angle)+up*math.sin(angle)))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(centers)-1):
        for j in range(n):faces.append((i*n+j,i*n+(j+1)%n,(i+1)*n+(j+1)%n,(i+1)*n+j))
    faces.append(tuple(range((len(centers)-1)*n,len(centers)*n)))
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj)
    return obj


def main():
    asset='croisement01-east-fallen-branch';workspace=OUT/'branch-round-1/assets'/asset
    if workspace.exists():raise FileExistsError(workspace)
    directory=OUT/'branch-domains';directory.mkdir(exist_ok=True)
    inv=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in inv['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    inventory=directory/'native-masks.json';inventory.write_text(json.dumps(inv,indent=2)+'\n')
    masks=directory/'east-fallen-branch.json'
    masks.write_text(json.dumps(dict(version=1,mask_inventory=str(inventory),projections=dict(exterior=dict(
        state='Initial static source',source_sha256=sha(OUT/'baseline/covered.png'),
        assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[70])]))),indent=2)+'\n')
    review=directory/'grouping-review.json'
    review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',asset_id=asset,
        catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'grouped-inventory/inventory.json'),
        evidence='Native mask70 and part68 are the source-visible bent fallen branch and upward twig. Native context and coordinate grid inspected.'),indent=2)+'\n')
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    prepare(workspace,asset_id=asset,scene_name='Croisement01 Refinement',collection_name='Croisement01 Working',
        source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',
        inventory_path=OUT/'grouped-inventory/inventory.json',review_path=review,
        source_mask_manifest=masks,width=256,height=256,framing_padding=1.16,
        lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    original=next(o for o in bpy.data.collections['Croisement01 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset)
    body=tube('Continuous main branch',MAIN);twig=tube('Upward twig union operand',TWIG)
    bpy.context.view_layer.objects.active=body;body.select_set(True)
    mod=body.modifiers.new('Continuous branch joint','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=twig
    bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(twig,do_unlink=True)
    mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True)
    inverse=original.matrix_world.inverted()
    for vert in mesh.vertices:vert.co=inverse@vert.co
    for mat in original.data.materials:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name='Source UV');ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=ownership
    for face in mesh.polygons:
        for loop in face.loop_indices:
            p=original.matrix_world@mesh.vertices[mesh.loops[loop].vertex_index].co
            uv.data[loop].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);ownership.data[loop].color=(0,1,1,1)
    original.data=mesh
    bm=bmesh.new();bm.from_mesh(mesh)
    topology=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    if topology['nonmanifold_edges'] or topology['degenerate_faces']:raise ValueError(topology)
    validate(workspace);modified(workspace)
    inspection=workspace/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private candidate; self-review pending',main_trace=MAIN,twig_trace=TWIG,topology=topology,model_sha256=sha(workspace/'model.blend'),limitations=['Native source traces approximate centerlines. Radius, hidden depth and twig height are inferred.','Independent native source coverage, ground contacts and actual material review pending.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(workspace)];render_candidate.main();release()

if __name__=='__main__':main()
