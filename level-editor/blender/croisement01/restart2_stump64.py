"""Refine the southwest broken stump with a single continuous shaft and native cap outline."""
import json
import shutil
import math
import numpy as np
import sys
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).parent))
from prepare_props import stump, OUT
from refinement_workspace import prepare, modified, validate
from refinement_inventory import inventory
from render_slots import acquire
from evidence_io import sha

def main():
    if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
    dest=OUT/'restart2/stump64-wood-v4';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement01 Working'];asset='croisement01-southwest-broken-stump'
    obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='building-060')
    obj['asset_group']=asset
    from restart2_tree18 import tube,union,assign_mesh,SIN,COS
    from mathutils import Vector
    import bmesh
    # Full rounded wood depth is inferred independently from the visible jagged rim.
    cap=[]
    for i in range(24):
        angle=math.tau*i/24
        x=467+16*math.cos(angle);y=-791/SIN+18.5*math.sin(angle)
        z=82-.20*(x-467)+1.3*math.sin(5*angle)+.7*math.cos(7*angle)
        cap.append(dict(x=x,y=-y*SIN,z_top=z*COS,z_bottom=0))
    record=dict(points=cap)
    stump(obj,record,60,profile=(465,791,1.02,.96,.92))
    vertices=[obj.matrix_world@v.co for v in obj.data.vertices];faces=[tuple(f.vertices) for f in obj.data.polygons];rim=faces.pop();center=sum((vertices[i] for i in rim),Vector())/len(rim)
    # A shallow broken perimeter surrounds a coherent central wood surface.
    inner=[]
    for index in rim:
        point_on_cap=center.lerp(vertices[index],.60)
        point_on_cap.z=81-.20*(point_on_cap.x-467)
        inner.append(len(vertices));vertices.append(point_on_cap)
    faces.extend((rim[i],rim[(i+1)%len(rim)],inner[(i+1)%len(rim)],inner[i]) for i in range(len(rim)))
    faces.append(tuple(inner))
    mesh=bpy.data.meshes.new('Broken stump world core');mesh.from_pydata(vertices,[],faces);mesh.update()
    body=bpy.data.objects.new('Broken stump connected core',mesh);bpy.context.scene.collection.objects.link(body)
    def point(x,y,h):return Vector((x,-(y+h)/SIN,h/COS))
    branch=tube('Short snapped left stub',[point(455,749,40),point(444,737,52),point(432,719,67)],[7.3,6.5,6.0])
    union(body,branch)
    root=tube('Short observed right root',[point(475,771,19),point(482,777,13),point(485,789,4)],[5.8,4.5,2.8]);union(body,root)
    final=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(obj,final)
    bm=bmesh.new();bm.from_mesh(obj.data);report=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    if report['nonmanifold_edges'] or report['degenerate_faces']:raise ValueError(report)
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==64)
    from PIL import Image,ImageDraw,ImageChops
    alpha=Image.open(row['png']).convert('L');core=Image.new('L',alpha.size)
    # Native broken cap and short left stub are wood; broad lower foreground ivy remains separately owned.
    draw=ImageDraw.Draw(core)
    draw.polygon([(31,1),(38,0),(43,6),(48,6),(56,11),(60,18),(57,31),(55,57),(63,64),(63,77),(57,80),(48,72),(43,80),(35,84),(32,72),(34,58),(31,43),(29,27),(24,20),(25,12)],fill=255)
    draw.polygon([(3,4),(8,3),(13,7),(15,14),(21,17),(26,20),(30,28),(24,31),(15,25),(9,20),(5,12)],fill=255)
    wood=ImageChops.multiply(alpha,core)
    left,top=row['box_top_left'];rgb=np.asarray(Image.open(OUT/'baseline/covered.png').convert('RGB').crop((left,top,left+alpha.width,top+alpha.height))).astype(float)
    # Mixed yellow/green boundary samples are deferred with ivy instead of painting leaves onto wood.
    leafy=(rgb[:,:,1]>.85*rgb[:,:,0])&(rgb[:,:,1]>1.7*rgb[:,:,2])&(rgb[:,:,1]>95)
    owned=np.asarray(wood).copy();mixed_boundary_count=int(((owned>0)&leafy).sum());owned[leafy]=0
    wood=Image.fromarray(owned);domain=dest/'wood-domain.png';wood.save(domain)
    ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
    # Remove unsupported high teeth outside the native source-camera upper contour while keeping full hidden depth.
    coverage=np.asarray(alpha)>0;columns=[];tops=[]
    for column in range(coverage.shape[1]):
        ys=np.flatnonzero(coverage[:,column])
        if len(ys):columns.append(left+column+.5);tops.append(top+ys[0])
    inverse=obj.matrix_world.inverted();clamped=0
    for vertex in obj.data.vertices:
        world=obj.matrix_world@vertex.co
        if world.z<50:continue
        ceiling=float(np.interp(world.x,columns,tops))-.2
        screen_y=-world.y*SIN-world.z*COS
        if screen_y<ceiling:
            world.z=(-world.y*SIN-ceiling)/COS;vertex.co=inverse@world;clamped+=1
    obj.data.update()
    uv=obj.data.uv_layers['Source UV']
    for loop in obj.data.loops:
        world=obj.matrix_world@obj.data.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(world.x/1408,1-(-world.y*SIN-world.z*COS)/960)
    report['upper_contour_clamped_vertices']=clamped

    native['masks'].append(dict(row,index=264,png=str(domain)))
    masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n')
    source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial southwest broken stump wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[264])]))),indent=2)+'\n')
    catalog=dest/'catalog.json';groups=json.loads((OUT/'catalog.json').read_text())
    terrain={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
    keep={o for o in collection.all_objects if o.type=='MESH' and (o==obj or o.get('source_node') in terrain)}
    for other in list(bpy.data.objects):
        if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    nodes={o.get('source_node') for o in keep};retained=[]
    for group in groups['groups']:
        parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
        if parts:retained.append(dict(group,parts=parts))
    groups['groups']=retained
    groups['canonical_owners']={f"building-{part['obstacle']:03}" if 'obstacle' in part else part['node']:group['id'] for group in groups['groups'] for part in group['parts']}
    catalog.write_text(json.dumps(groups,indent=2)+'\n')
    inventory(dest/'inventory',collection_name=collection.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    grouping=dest/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask64 and original part60 define a broken stump with a short left stub. Conservative exposed cap and left bark domain excludes right and lower ivy. Compact context retains only stump and provisional terrain; inferred full shaft continues behind the ivy.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=collection.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog,inventory_path=dest/'inventory/inventory.json',review_path=grouping,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    (worker/'inspection/construction.json').write_text(json.dumps(dict(status='private candidate; actual/source/contact review required',model_sha256=sha(worker/'model.blend'),wood_domain_sha256=sha(domain),geometry=report,scope='Wood only; dense right and lower ivy is separately deferred. Original part60 gameplay reference remains authoritative; neutral hidden wood is not observed source.',lower_to_cap_radius=1.02,whole_map_duplicate=False,mixed_boundary_pixels_deferred=mixed_boundary_count),indent=2)+'\n')
    import render_candidate
    sys.argv=['render','--',str(worker)];render_candidate.main()

if __name__=='__main__':main()
