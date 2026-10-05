"""Eastern leaf-obscured stem with a conservative bark domain."""
import argparse
import json
import math
import random
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageChops

sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import ROOT,OUT,SIN,COS,tube,union,assign_mesh,fit_native_width
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    dest=OUT/f'restart2/tree22-v{args.revision}';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-22';name='Eastern Leaf-Obscured Forest Tree'
    obj=next(o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='building-075')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==22)
    alpha=Image.open(row['png']).convert('L')
    from PIL import ImageDraw
    core=Image.new('L',alpha.size)
    ImageDraw.Draw(core).polygon([(4,0),(19,0),(19,18),(15,25),(14,42),(12,55),(13,77),(14,94),(15,111),(14,126),(12,143),(11,157),(9,167),(5,170),(5,148),(7,128),(8,106),(7,88),(6,66),(5,44),(4,20)],fill=255)
    wood=ImageChops.multiply(alpha,core);deferred=ImageChops.subtract(alpha,wood)
    wood.save(dest/'wood-domain.png');deferred.save(dest/'deferred-leaf-domain.png')
    native['masks'].append(dict(row,index=222,png=str(dest/'wood-domain.png')))
    (dest/'source-ownership.json').write_text(json.dumps(dict(status='private conservative bark-domain candidate; semantic visual review required',native_mask=22,wood_mask=222,known_bark_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(deferred))),reason='Manual left-side continuous bark domain. Dense right-side native leaf silhouettes remain deferred, rather than projecting them as bark.'),indent=2)+'\n')
    catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
    for group in catalog['groups']:
        remaining=[p for p in group['parts'] if p.get('obstacle')!=75]
        if remaining:groups.append(dict(group,parts=remaining))
    crown_node='foliage-tree22-inferred-crown'
    groups.append(dict(id=asset,name=name,parts=[dict(obstacle=75,name='Shaded stem'),dict(node=crown_node,name='Inferred off-map crown')],status='private candidate'))
    catalog['groups']=groups;catalog['version']=2;catalog['canonical_owners']={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']}
    catalog_path=dest/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    obj['asset_group']=asset;obj['asset_name']=name
    base_y=-181/SIN;trace=[(1330,177,4),(1337,163,11),(1339,148,13.5),(1338,130,12),(1338,111,11),(1337,91,11),(1335,70,10.5),(1334,50,10),(1333,30,9.5),(1332,10,9),(1331,-10,9)]
    centers=[Vector((x,base_y,(181-y)/COS)) for x,y,r in trace]+[Vector((1333,base_y-4,285)),Vector((1328,base_y-7,350)),Vector((1332,base_y-9,416))]
    body=tube('Rounded shaded stem',centers,[r for x,y,r in trace]+[8.5,5,1]);body['defer_union']=True
    rng=random.Random(202201)
    for i in range(8):
        a=math.tau*i/8;start=Vector((1332,base_y-6,320+i*5));tip=Vector((1332+math.cos(a)*78,base_y-7+math.sin(a)*90,408+rng.uniform(-12,30)))
        union(body,tube(f'Inferred bough{i}',[start,start.lerp(tip,.5)+Vector((0,0,10)),tip],[5,3,.6]))
    bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Connected hidden branches','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.7;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
    mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(obj,mesh)
    fit_domain=np.asarray(alpha)>127
    if args.revision>=3:
        # Infer the hidden wood contour from the continuous visible bark strip;
        # surrounding leaf tips do not establish the trunk's outside boundary.
        fit_domain=np.zeros_like(fit_domain)
        for y,line in enumerate(np.asarray(core)>127):
            columns=np.flatnonzero(line)
            if len(columns):fit_domain[y,max(0,columns[0]-1):min(len(line),columns[-1]+7)]=True
        Image.fromarray((fit_domain*255).astype('uint8')).save(dest/'inferred-wood-contour.png')
    fit=fit_native_width(obj,fit_domain,'single',x0=row['box_top_left'][0]);(dest/'source-contour-fit.json').write_text(json.dumps(fit,indent=2)+'\n')
    removed_islands=[]
    if args.revision>=1:
        bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);components=[]
        while remaining:
            queue=[remaining.pop()];component=[]
            while queue:
                vert=queue.pop();component.append(vert)
                for edge in vert.link_edges:
                    other=edge.other_vert(vert)
                    if other in remaining:remaining.remove(other);queue.append(other)
            components.append(component)
        components.sort(key=len,reverse=True)
        for component in components[1:]:
            points=[obj.matrix_world@v.co for v in component]
            xmin,xmax=min(p.x for p in points),max(p.x for p in points)
            ymin,ymax=min(-p.y*SIN-p.z*COS for p in points),max(-p.y*SIN-p.z*COS for p in points)
            pixel_columns=list(range(math.ceil(xmin-.5),math.floor(xmax-.5)+1))
            pixel_rows=list(range(math.ceil(ymin-.5),math.floor(ymax-.5)+1))
            if len(component)>16 or (pixel_columns and pixel_rows):
                raise ValueError(f'Detached wood covers native sample centers: {len(component)} vertices, {pixel_columns}, {pixel_rows}')
            removed_islands.append(dict(vertices=len(component),native_bounds=[xmin,ymin,xmax,ymax],native_sample_centers=0,reason='Subpixel voxel island whose projected bounds contain no native pixel sample center.'))
            bmesh.ops.delete(bm,geom=component,context='VERTS')
        bm.to_mesh(obj.data);bm.free()
    verts=[];faces=[];lobes=[(-53,-15,400,43,60,45),(49,20,415,46,62,48),(-4,-55,437,49,55,39),(-20,45,448,48,64,38),(25,-12,464,36,49,26)]
    for i in range(1150):
        theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;dx,dy,z,rx,ry,rz=lobes[i%len(lobes)]
        center=Vector((1332+dx+rx*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-8+dy+ry*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),z+rz*rad*zeta))
        size=rng.uniform(3,7);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized()
        n=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(n,n+1,n+2),(n,n+2,n+3)])
    mesh=bpy.data.meshes.new('Complete inferred tree22 crown');mesh.from_pydata(verts,[],faces);mesh.update()
    mat=bpy.data.materials.new('Unknown off-map tree22 foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
    crown=bpy.data.objects.new('Tree22 inferred off-map crown',mesh);working.objects.link(crown)
    for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred off-map crown',projection_component='crown').items():crown[k]=v
    if args.revision>=2:
        vertices=[];faces=[]
        terrain_nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
        for support in working.all_objects:
            if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
            offset=len(vertices);vertices.extend(support.matrix_world@v.co for v in support.data.vertices)
            faces.extend(tuple(offset+i for i in f.vertices) for f in support.data.polygons)
        terrain=BVHTree.FromPolygons(vertices,faces);ray=Vector((0,-COS,SIN))
        foot=min((obj.matrix_world@v.co for v in obj.data.vertices),key=lambda p:p.z)
        hit=terrain.ray_cast(foot+ray*5000,-ray,10000)[0]
        if hit is None:raise ValueError('Missing bank support beneath tree22 native root')
        shift=ray*((hit-foot).dot(ray)+.2)
        for target in (obj,crown):
            inverse=target.matrix_world.inverted()
            for vertex in target.data.vertices:vertex.co=inverse@(target.matrix_world@vertex.co+shift)
            target.data.update()
        (dest/'root-support-placement.json').write_text(json.dumps(dict(method='Move complete asset along original camera ray to measured archival bank; native image coordinates unchanged.',original_foot=list(foot),support=list(hit),shift=list(shift),status='Provisional contact; saved geometry and joint review required'),indent=2)+'\n')
        if args.revision>=5:
            foot=foot+shift
            # Replace the tiny basal termination with a connected support ring.
            # Keep the cut boundary exact; extend only its lower closure.
            bm=bmesh.new();bm.from_mesh(obj.data)
            for vertex in bm.verts:vertex.co=obj.matrix_world@vertex.co
            cut_z=foot.z+(30 if args.revision>=7 else 8)
            basal_radius=6.5 if args.revision>=8 else (8.5 if args.revision>=7 else 6.5)
            bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=Vector((0,0,cut_z)),plane_no=Vector((0,0,1)),clear_inner=True,clear_outer=False)
            boundary=[edge for edge in bm.edges if edge.is_boundary]
            if not boundary or any(abs(v.co.z-cut_z)>.01 for e in boundary for v in e.verts):raise ValueError('Unexpected basal cut boundary')
            vertices={v for e in boundary for v in e.verts}
            neighbors={v:[] for v in vertices}
            for edge in boundary:
                a,b=edge.verts;neighbors[a].append(b);neighbors[b].append(a)
            if any(len(adjacent)!=2 for adjacent in neighbors.values()):raise ValueError('Basal cut is not a closed ring')
            ring=[next(iter(vertices))];previous=None
            while True:
                following=next(v for v in neighbors[ring[-1]] if v!=previous)
                if following==ring[0]:break
                if following in ring:raise ValueError('Basal ring repeats before closure')
                previous=ring[-1];ring.append(following)
            if len(ring)!=len(vertices):raise ValueError('Basal cut has multiple loops')
            center=sum((v.co for v in ring),Vector())/len(ring)
            lower=[]
            for vertex in ring:
                direction=Vector((vertex.co.x-center.x,vertex.co.y-center.y,0)).normalized()
                support_center=foot if args.revision>=8 else center
                x=support_center.x+direction.x*basal_radius;y=support_center.y+direction.y*basal_radius
                support=terrain.ray_cast(Vector((x,y,cut_z+40)),Vector((0,0,-1)),100)[0]
                if support is None:raise ValueError(f'Missing continuous root-ring support at {x},{y}; cut={cut_z}; foot={list(foot)}')
                lower.append(bm.verts.new((x,y,support.z-.4)))
            for j in range(len(ring)):
                k=(j+1)%len(ring);bm.faces.new((ring[j],lower[j],lower[k],ring[k]))
            bm.faces.new(tuple(reversed(lower)))
            bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
            if any(not e.is_manifold for e in bm.edges):raise ValueError('Continuous root ring is not closed')
            inverse=obj.matrix_world.inverted()
            for vertex in bm.verts:vertex.co=inverse@vertex.co
            bm.to_mesh(obj.data);bm.free();obj.data.update()
            (dest/'basal-flare.json').write_text(json.dumps(dict(method='Replace basal point closure with a continuous bank-conforming ring; retain the existing cut boundary exactly.',cut_height=cut_z,ring_vertices=len(ring),radius=basal_radius,penetration=.4,status='Private candidate; native coverage and contact review required'),indent=2)+'\n')
        elif args.revision>=4:
            foot=foot+shift
            verts=[];faces=[];count=24
            for height,radius in [(0,6.5),(5,7.),(16,5.)]:
                for j in range(count):
                    angle=math.tau*j/count
                    x=foot.x+radius*math.cos(angle);y=foot.y+radius*math.sin(angle)
                    support=terrain.ray_cast(Vector((x,y,foot.z+40)),Vector((0,0,-1)),100)[0]
                    if support is None:raise ValueError('Missing local basal flare support')
                    z=support.z-.4 if height==0 else foot.z+height
                    verts.append(Vector((x,y,z)))
            faces.append(tuple(reversed(range(count))))
            for k in range(2):
                for j in range(count):faces.append((k*count+j,k*count+(j+1)%count,(k+1)*count+(j+1)%count,(k+1)*count+j))
            faces.append(tuple(range(2*count,3*count)))
            root_mesh=bpy.data.meshes.new('Continuous bank root flare');root_mesh.from_pydata(verts,[],faces);root_mesh.update()
            bm=bmesh.new();bm.from_mesh(root_mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(root_mesh);bm.free()
            root=bpy.data.objects.new('Continuous bank root flare',root_mesh);bpy.context.scene.collection.objects.link(root);union(obj,root)
            (dest/'basal-flare.json').write_text(json.dumps(dict(method='Connected short rounded flare whose lower ring follows measured local bank; shallow seating replaces a point contact.',foot=list(foot),radii=[6.5,7.,5.],heights=[0,5,16],support_penetration=.4,scope='Inferred basal wood behind deferred source foliage; known bark remains authoritative'),indent=2)+'\n')
    uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'input.blend'))
    inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native_path=dest/'masks.json';native_path.write_text(json.dumps(native,indent=2)+'\n')
    masks=dest/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(native_path),projections=dict(exterior=dict(state='Initial native tree22 conservative bark',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[222])]))),indent=2)+'\n')
    review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog_path),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask22 and part75 describe the eastern thin stem. Only a conservative visible bark strip is assigned; surrounding leaves remain deferred. Complete off-map crown inferred.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=masks,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private candidate; self-review pending',model_sha256=sha(worker/'model.blend'),native_mask=22,native_parts=[75],trace=trace,removed_subpixel_voxel_islands=removed_islands,reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['Crown, upper boughs and hidden stem depth are inferred.','Dense native foreground leaf domain remains deferred; only conservative bark ownership is mapped.','Terrain contact, actual crown depth and native coverage need independent saved-model review.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(worker)];render_candidate.main()


if __name__=='__main__':main()
