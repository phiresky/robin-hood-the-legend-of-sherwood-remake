"""Private tree25 volume using native foliage and the two permitted tree constructions."""
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
                str(ROOT / 'level-editor/refinement/blender'), str(ROOT / 'level-editor/blender/leicester')]
from render_slots import acquire, release
from refinement_workspace import prepare, modified
import foliage_trees

OUT = ROOT / 'level-editor/work/croisement03-refinement'
ASSET = 'croisement03-tree-25'
SINE, COSINE = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = Vector((0, -COSINE, SINE))
# Source x/y, inferred circular radius, inferred displacement along the source ray.
PATHS = [
    [(1265,800,17,0),(1265,780,18,0),(1264,767,17.5,0),(1263,744,18,0),
     (1277,720,13,0),(1286,694,10,-5),(1288,670,8,-15),(1294,646,4.8,-25),
     (1297,625,3,-35),(1299,606,.8,-45)],
    [(1263,744,17,0),(1246,724,12,15),(1232,709,11,30),(1210,700,8,45),
     (1187,690,4,65),(1165,680,1,80)],
    [(1232,709,7,30),(1213,691,6,45),(1200,681,5,60),(1191,670,5,75),(1187,658,1.5,90)],
    [(1283,704,6,-3),(1265,697,5,-25),(1250,687,3.5,-50),(1235,676,2,-75),(1234,652,.7,-95)],
    [(1280,711,8,-2),(1303,699,5,-20),(1326,687,4,-40),(1348,685,2.8,-65),(1370,681,1,-85)],
]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def crown_packet(root, source, level):
    record = level['masks'][116]
    x, y = record['box_top_left']
    alpha_path = OUT / 'baseline/masks/000116.png'
    alpha = np.array(Image.open(alpha_path).convert('L')) > 127
    height, width = alpha.shape
    rgb = np.array(source.convert('RGB').crop((x, y, x + width, y + height)))
    yy, xx = np.mgrid[:height, :width]
    sx, sy = xx + x, yy + y
    seeds = np.array([(1189,567),(1261,558),(1342,585),(1140,638),
                      (1218,637),(1307,658),(1382,683),(1328,732)])
    distances = (sx[:,:,None] - seeds[:,0]) ** 2 + (sy[:,:,None] - seeds[:,1]) ** 2
    labels = distances.argmin(axis=2)
    output = root / 'crown-source'
    output.mkdir()
    lobes, union = [], np.zeros_like(alpha)
    depth_radii = [250,250,210,100,110,100,90,65]
    for number, (cx, cy) in enumerate(seeds):
        owned = alpha & (labels == number)
        radius = math.sqrt(float(distances[:,:,number][owned].max())) * 1.2
        angle = np.arctan2(sy - cy, sx - cx)
        scallop = .94 + .035 * np.sin(angle * 11 + number) + .025 * np.cos(angle * 17 - number)
        support = alpha & (np.sqrt(distances[:,:,number]) < radius * scallop)
        union |= support
        ys, xs = np.nonzero(support)
        x0, x1 = max(0,int(xs.min())-2), min(width,int(xs.max())+3)
        y0, y1 = max(0,int(ys.min())-2), min(height,int(ys.max())+3)
        rgba = np.zeros((y1-y0,x1-x0,4), dtype=np.uint8)
        rgba[:,:,:3] = rgb[y0:y1,x0:x1]
        rgba[:,:,3] = support[y0:y1,x0:x1] * 255
        rgba[rgba[:,:,3] == 0,:3] = 0
        front = output / f'lobe-{number:02}-native.png'
        Image.fromarray(rgba).save(front)
        rgba[:,:,:3] = 105
        rgba[rgba[:,:,3] == 0,:3] = 0
        back = output / f'lobe-{number:02}-unknown.png'
        Image.fromarray(rgba).save(back)
        lobes.append(dict(index=number,bbox_source=[x+x0,y+y0,x+x1,y+y1],
                          source=str(front),unknown=str(back),source_sha256=sha(front),
                          unknown_sha256=sha(back),depth_radius=depth_radii[number]))
    assert np.array_equal(union, alpha)
    # Complete the clipped east edge as a separate, explicitly unknown lobe.
    x0,y0,x1,y1 = 1350,650,1460,778
    yy,xx = np.mgrid[y0:y1,x0:x1]
    dx,dy = (xx-1406)/54,(yy-712)/61
    angle = np.arctan2(dy,dx)
    support = np.hypot(dx,dy) < (.87+.07*np.sin(angle*11)+.04*np.cos(angle*17))
    support &= ((np.sin(xx*.63)*np.cos(yy*.49)) < .9)
    # Inside the source map, this hypothesis may not cover visible source gaps.
    native_support = np.zeros_like(support)
    lx,ty,rx,by = max(x0,x),max(y0,y),min(x1,x+width),min(y1,y+height)
    native_support[ty-y0:by-y0,lx-x0:rx-x0] = alpha[ty-y:by-y,lx-x:rx-x]
    support &= (xx >= source.width) | native_support
    rgba = np.zeros((*support.shape,4),dtype=np.uint8)
    rgba[:,:,:3] = 105; rgba[:,:,3] = support * 255; rgba[~support,:3] = 0
    inferred = output / 'lobe-08-off-map-unknown.png'
    Image.fromarray(rgba).save(inferred)
    lobes.append(dict(index=8,bbox_source=[x0,y0,x1,y1],source=str(inferred),unknown=str(inferred),
                      observed=False,source_sha256=sha(inferred),unknown_sha256=sha(inferred),depth_radius=85))
    evidence = dict(source_rgb_sha256=sha(OUT/'baseline/covered.png'),native_alpha_sha256=sha(alpha_path),
                    native_mask=116,source_node='building-046',lobes=lobes,
                    observed_pixels=int(alpha.sum()),native_union_preserved=True,
                    interpretation='Native view-occlusion foliage supplies the static front coverage; this is distinct from animated wind-frame alpha.',
                    inferences=['Lobe separation and source-ray depth are inferred.',
                                'East continuation beyond the image boundary is unknown, not clipped.',
                                'Wind animation must be integrated separately; no animated-state completion claim.'],
                    permitted_external_construction_references=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'])
    write(output/'source-partition.json',evidence)
    return evidence


def wood_mesh():
    vertices, faces, sides = [], [], 16
    for path in PATHS:
        centers = [Vector((x,-800/SINE,(800-y)/COSINE)) + RAY*depth for x,y,radius,depth in path]
        start = len(vertices)
        for i, (center, point) in enumerate(zip(centers,path)):
            axis = (centers[min(i+1,len(path)-1)]-centers[max(0,i-1)]).normalized()
            side = axis.cross(RAY).normalized(); up = axis.cross(side).normalized()
            for k in range(sides):
                angle = k*math.tau/sides
                vertices.append(tuple(center+point[2]*(1+.025*math.cos(angle*5))*(side*math.cos(angle)+up*math.sin(angle))))
        faces.append(tuple(start+k for k in reversed(range(sides))))
        for i in range(len(path)-1):
            for k in range(sides):
                faces.append((start+i*sides+k,start+i*sides+(k+1)%sides,start+(i+1)*sides+(k+1)%sides,start+(i+1)*sides+k))
        faces.append(tuple(start+(len(path)-1)*sides+k for k in range(sides)))
    mesh = bpy.data.meshes.new('Tree25 source-traced depth branches')
    mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    return mesh


def main():
    root = OUT / 'restart2/tree25-full-v8'
    root.mkdir(exist_ok=False)
    worker = root/'assets'/ASSET
    source = Image.open(OUT/'baseline/covered.png').convert('RGBA')
    level = json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    def native(index):
        mask=Image.new('L',source.size)
        mask.paste(Image.open(OUT/f'baseline/masks/{index:06}.png'),tuple(level['masks'][index]['box_top_left']))
        return mask
    domain = native(25)
    for number in [70,113,116]:
        domain = ImageChops.subtract(domain,native(number))
    ImageDraw.Draw(domain).rectangle((0,792,source.width,source.height),fill=0)
    domain.save(root/'observed-bark.png')
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    masks['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-bark.png'),
                             box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,obstacle_indices=[46]))
    write(root/'mask-inventory.json',masks)
    write(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),
         projections={'exterior':dict(state='Native wood excludes foreground wall/leaves; physical crown has separate opacity and ownership',
         source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
    evidence = crown_packet(root,source,level)
    write(root/'source-trace.json',dict(paths=PATHS,ground_source_y=800,
         depth='Branch and crown depth is an explicit source-ray inference; upper branch row centers/widths measured against native25.',
         limitations=['Native116 is view-occlusion coverage, not wind animation alpha.','Tree-wall contact and full source/material review pending.']))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'))
        bpy.context.preferences.filepaths.save_version=0
        collection=bpy.data.collections['Croisement03 Working']
        obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==ASSET)
        obj.data=wood_mesh();obj.matrix_world.identity()
        bpy.context.view_layer.objects.active=obj;obj.select_set(True)
        modifier=obj.modifiers.new('Fused circular branch junctions','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.65
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        bm=bmesh.new();bm.from_mesh(obj.data)
        topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces))
        bm.free();assert not any(topology.values())
        obj.data.uv_layers.new(name='UVMap')
        material=bpy.data.materials.new('Tree25 unknown wood');material.diffuse_color=(.42,.42,.42,1);obj.data.materials.append(material)
        crown=bpy.data.objects.new('Tree25 native static crown',bpy.data.meshes.new('Tree25 crown placeholder'))
        collection.objects.link(crown);crown['asset_group']=ASSET;crown['source_node']='building-046';crown['projection_component']='native-static-crown-116'
        foliage_trees.CONFIG[46]=dict(ground=800.)
        foliage_trees.DEPTHS=[0.,0.,0.,0.,0.,0.,0.,0.,-30.]
        foliage_trees.refine_crown(crown,46,evidence)
        bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);crown.select_set(True)
        bpy.context.view_layer.objects.active=obj;bpy.ops.object.join()
        # Joining remaps material slots; update the face fallback indices as well.
        fallback=obj.data.attributes.get('reprojection_fallback_material')
        assert fallback is not None
        for polygon in obj.data.polygons:
            fallback.data[polygon.index].value=polygon.material_index
        # Freeze the whole new construction as its private framing baseline.
        bounds=[max(v.co[i] for v in obj.data.vertices)-min(v.co[i] for v in obj.data.vertices) for i in range(3)]
        assert bounds[1]>=bounds[0], bounds
        prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name=collection.name,
                source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',
                review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=384,
                framing_padding=1.25,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        modified(worker)
        inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
        write(inspection/'construction.json',dict(status='PRIVATE HOLD: complete static volume candidate; actual materials, native coverage, wall contact, wind state and independent review pending',
              model_sha256=sha(worker/'model.blend'),wood_topology=topology,bounds_xyz=bounds,depth_at_least_width=True,
              construction_frame='Private full-volume construction used to freeze cameras; native obstacle represented only the lower trunk.',
              source_mask_semantics=evidence['interpretation']))
        print(worker)
    finally:
        release()


if __name__=='__main__':
    main()
