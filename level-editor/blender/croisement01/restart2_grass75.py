"""Private rooted-grass comparison retaining each native source pixel."""
import json
import argparse
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
sys.path.insert(0, str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha
from render_slots import acquire
from refinement_workspace import prepare, modified, validate
from refinement_inventory import inventory


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mask',type=int,choices=[74,75,76,82],default=75);parser.add_argument('--revision',type=int,default=12);parser.add_argument('--construction',choices=['generic','leaf-path','leaf-curve','leaf-thin'],default='generic');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    mask=args.mask
    acquire()
    dest = OUT/f'restart2/grass{mask}-volume-v{args.revision}'
    dest.mkdir(exist_ok=False)
    source = dest/'source'; source.mkdir()
    branch = OUT/'restart2/branch-source-fit-v3/branch-round-10/assets/croisement01-east-fallen-branch'
    paths = OUT/f'restart2/grass{mask}-source-v1/grass-{mask:03}-leaf-paths-v3/paths.json'
    leaves = json.loads(paths.read_text())
    native = json.loads((OUT/'baseline/masks/manifest.json').read_text())
    row = next(r for r in native['masks'] if r['index']==mask)
    x,y,w,h = row['box_top_left']+row['box_size']
    rgba = Image.open(OUT/'baseline/covered.png').convert('RGBA').crop((x,y,x+w,y+h))
    alpha = Image.open(OUT/'baseline/masks'/row['png']).convert('L')
    rgba.putalpha(alpha); rgba.save(source/'native.png'); alpha.save(source/'domain.png')
    bpy.ops.wm.open_mainfile(filepath=str(branch/'model.blend'))
    bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working']
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    ray=Vector((0,-cosine,sine))
    terrain_nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
    tv,tf=[],[]
    for obj in working.all_objects:
        if obj.type!='MESH' or obj.get('source_node') not in terrain_nodes: continue
        offset=len(tv);tv.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        tf.extend(tuple(offset+i for i in p.vertices) for p in obj.data.polygons)
    terrain=BVHTree.FromPolygons(tv,tf)
    root_x=x+leaves['root_source_local'][0]; root_y=y+leaves['root_source_local'][1]
    target=Vector((root_x,-root_y/sine,0))
    root=terrain.ray_cast(target+ray*5000,-ray,10000)[0]
    if root is None: raise ValueError('Missing source-ray terrain support')
    sys.path.append(str(ROOT/'level-editor/blender/croisement02'))
    from tree_geometry import material, one_sided
    mats=[material(f'Grass{mask} native front',source/'native.png',True),
          material(f'Grass{mask} inferred reverse',source/'native.png',False)]
    pixels=np.asarray(rgba)
    observed_pixels=pixels[pixels[:,:,3]>127]
    # Bright yellow-green samples are material cues from this exact tuft;
    # shadow/soil samples are not painted onto newly inferred leaf backs.
    leaf_pixels=observed_pixels[(observed_pixels[:,1]>70)&(observed_pixels[:,1]>observed_pixels[:,2]*1.5)&(observed_pixels[:,0]>55)]
    if len(leaf_pixels)<8:raise ValueError('Too few own-tuft leaf material samples')
    Image.fromarray(leaf_pixels.reshape(1,-1,4)).save(source/'inferred-leaf-palette.png')
    mats.append(material(f'Grass{mask} inferred additional leaf backs',source/'inferred-leaf-palette.png',False))
    mats.append(material(f'Grass{mask} native-projected additional fronts',source/'native.png',True))
    for mat in mats:
        one_sided(mat)
        mat['texture_provenance']=f'Observed Croisement01 grass{mask} front' if mat.get('foliage_observed') else f'Inferred reverse using only this Croisement01 grass{mask} source'
        if mat in (mats[0],mats[1],mats[3]):
            for shader_node in mat.node_tree.nodes:
                if shader_node.type=='TEX_IMAGE':shader_node.extension='CLIP'
    wood=next(o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')=='croisement01-east-fallen-branch')
    wood_tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[tuple(p.vertices) for p in wood.data.polygons])
    factors={leaf:.8+.3*math.cos(leaf*2.399963229728653) for leaf in range(leaves['leaf_count'])}
    offsets={leaf:0. for leaf in factors}
    constraints=[]
    for pixel in leaves['observed_pixels']:
        px=x+pixel['x']+.5;py=y+pixel['y']+.5
        origin=Vector((px,-py/sine,0))+ray*5000
        hit=wood_tree.ray_cast(origin,-ray,10000)[0]
        if hit is None:continue
        lift=max(.5,(root_y-py)/cosine)
        progress=min(1.,lift/3.)
        correction=max(0.,hit.z+.7-root.z-.08-lift*factors[pixel['leaf']])/progress
        offsets[pixel['leaf']]=max(offsets[pixel['leaf']],correction)
        constraints.append(dict(x=px,y=py,leaf=pixel['leaf'],wood_z=hit.z,required_offset=correction))
    if max(offsets.values())>30:raise ValueError(f'Foreground depth conflict needs explicit geometry review: {max(offsets.values())}')
    bends={}
    for leaf in factors:
        required=[q for q in constraints if q['leaf']==leaf]
        if not required:continue
        height=max(32.,max(q['wood_z']+.8-root.z for q in required)+12.)
        rate=max(.035,max(-math.log(1-(q['wood_z']+.8-root.z)/height)/max(.5,(root_y-q['y'])/cosine) for q in required))
        bends[leaf]=(height,rate)
    vertices=[];faces=[];uvs=[];slots=[];owns=[];keys={};reverse_vertices={}
    def point(px,py,leaf):
        # One continuous bent depth field per inferred leaf, with shared
        # vertices inside each leaf. Adjacent pixels no longer float at
        # independently selected random depths.
        lift=max(0.,(root_y-py)/cosine)
        phase=leaf*2.399963229728653
        factor=factors[leaf]
        distance=min(1.,math.hypot(px-root_x,py-root_y)/max(1.,h*.65))
        if leaf in bends:
            height,rate=bends[leaf]
            z=root.z+.08+height*(1-math.exp(-lift*rate))
        else:
            z=root.z+.08+lift*factor+max(-.04,(px-root_x)*.12*math.sin(phase)*distance)
        return Vector((px,-(py+z*cosine)/sine,z))
    for pixel in leaves['observed_pixels']:
        px,py,leaf=pixel['x'],pixel['y'],pixel['leaf']
        ring=[]
        for dx,dy in ((0,0),(1,0),(1,1),(0,1)):
            key=(px+dx,py+dy,leaf)
            if key not in keys:
                keys[key]=len(vertices);vertices.append(point(x+px+dx,y+py+dy,leaf))
            ring.append(keys[key])
        for ids in ((0,1,2),(0,2,3)):
            face=tuple(ring[i] for i in ids)
            if (vertices[face[1]]-vertices[face[0]]).cross(vertices[face[2]]-vertices[face[0]]).dot(ray)<0: face=tuple(reversed(face))
            for flip in (False,True):
                if flip:
                    for index in face:
                        if index not in reverse_vertices:
                            reverse_vertices[index]=len(vertices)
                            vertices.append(vertices[index]-ray*.02)
                    f=tuple(reverse_vertices[index] for index in reversed(face))
                else:
                    f=face
                faces.append(f);slots.append(int(flip));owns.append(not flip)
                uvs.extend(((vertices[i].x-x)/w,1-(-vertices[i].y*sine-vertices[i].z*cosine-y)/h) for i in f)
    rng=np.random.default_rng(mask*100+5)
    def append_triangle(points,slot,coords,reverse=False):
        points=list(points);coords=list(coords)
        front=(points[1]-points[0]).cross(points[2]-points[0]).dot(ray)>0
        if front==reverse:points.reverse();coords.reverse()
        start=len(vertices);vertices.extend(points);faces.append((start,start+1,start+2));slots.append(slot);owns.append(slot==3);uvs.extend(coords)
    # Hidden blades form a radial tuft with connected tapered strips. Their
    # camera-facing sides remain clipped by the native domain; reverse sides
    # use only the local leaf palette and cannot invent observed front pixels.
    for blade in range(36):
        angle=blade*math.tau/36+rng.uniform(-.13,.13)
        outward=Vector((math.cos(angle),math.sin(angle),0));side=Vector((-outward.y,outward.x,0))
        length=w*rng.uniform(.25,.43);rise=h*rng.uniform(.55,1.05)
        base=root+outward*rng.uniform(0,1.4)+Vector((0,0,.15))
        centers=[base+outward*(length*t**1.4)+Vector((0,0,rise*math.sin(t*math.pi*.72))) for t in np.linspace(0,1,9)]
        color=int(rng.integers(len(leaf_pixels)));palette_uv=((color+.5)/len(leaf_pixels),.5)
        for j in range(8):
            width0=.46*(1-j/8)+.015;width1=.46*(1-(j+1)/8)+.015
            pts=[centers[j]-side*width0,centers[j]+side*width0,centers[j+1]+side*width1,centers[j+1]-side*width1]
            for ids in ((0,1,2),(0,2,3)):
                tri=[pts[i] for i in ids]
                append_triangle(tri,3,[((p.x-x)/w,1-(-p.y*sine-p.z*cosine-y)/h) for p in tri])
                append_triangle([p-ray*.01 for p in tri],2,[palette_uv]*3,reverse=True)
    mesh=bpy.data.meshes.new(f'Grass{mask} coherent curved leaf surfaces');mesh.from_pydata(vertices,[],faces);mesh.update()
    for mat in mats:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name='Foliage UV');ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=ownership
    for poly,slot,known in zip(mesh.polygons,slots,owns):
        poly.material_index=slot
        for loop in poly.loop_indices:
            uv.data[loop].uv=uvs[loop];ownership.data[loop].color=(float(known),1,1,1)
    asset=f'croisement01-grass-{mask}';node=f'foliage-native-grass{mask}';name={74:'Southwest Field Grass',75:'East Branch Foreground Grass',76:'Central Field Grass',82:'Southeast Field Grass'}[mask]
    obj=bpy.data.objects.new(name,mesh);working.objects.link(obj)
    for k,v in dict(source_node=node,asset_group=asset,asset_name=name,part_name='Rooted native leaves',projection_component='crown',projection_preserve=True,foliage_physical_opacity=True).items():obj[k]=v
    rgba.save(source/'complete-source.png');rgba.save(source/'observed-source.png')
    from ground_plant_geometry import build
    generic_report=build(obj,dict(directory=str(source),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=mask,ground_z=root.z))
    if args.construction in ('leaf-path','leaf-curve','leaf-thin'):
        if args.construction=='leaf-thin':
            from restart2_ground_blades_thin import build as build_blades
        elif args.construction=='leaf-curve':
            from restart2_ground_blades_curve import build as build_blades
        else:
            from restart2_ground_blades import build as build_blades
        generic_report=build_blades(obj,source,leaves,root.z)
    for mat in obj.data.materials:
        mat['texture_provenance']=f'Observed Croisement01 grass{mask} front' if mat.get('foliage_observed') else f'Inferred reverse using only this Croisement01 grass{mask} source'
    (source/'generic-construction.json').write_text(json.dumps(generic_report,indent=2)+'\n')
    catalog=json.loads((OUT/'catalog.json').read_text());catalog['version']=2
    catalog['canonical_owners']={f"building-{p['obstacle']:03}":g['id'] for g in catalog['groups'] for p in g['parts']}
    catalog['canonical_owners'][node]=asset
    catalog['groups'].append(dict(id=asset,name=name,parts=[dict(node=node,name=f'Native grass{mask}')],status='private candidate'))
    catalog_path=dest/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'input.blend'))
    inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    for entry in native['masks']:entry['png']=str(OUT/'baseline/masks'/entry['png'])
    native_path=dest/'masks.json';native_path.write_text(json.dumps(native,indent=2)+'\n')
    masks=dest/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(native_path),projections=dict(exterior=dict(state=f'Initial native grass{mask}',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,source_node=node,mask_indices=[mask])],occluder_constraints=[dict(reviewed=True,source_node=node,receiver_nodes=sorted({'ground',*catalog['canonical_owners']}-{node}),mask_indices=[mask],reason=f'Inferred foliage only occludes foreign receivers within native grass{mask}.')]))),indent=2)+'\n')
    grouping=dest/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog_path),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence=f'Native grass{mask} mask and untouched context identify this individual grass tuft. Hidden leaf depth is inferred; other groups provisional.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,inventory_path=dest/'inventory/inventory.json',review_path=grouping,source_mask_manifest=masks,width=256,height=256,framing_padding=1.25,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private inferred leaf geometry; no approval',native_mask=mask,leaf_count=leaves['leaf_count'],source_pixels=len(leaves['observed_pixels']),model_sha256=sha(worker/'model.blend'),final_geometry=generic_report,unused_intermediate_leaf_hypothesis=dict(root_world=list(root),foreground_factors=factors,foreground_offsets=offsets,foreground_constraints=constraints),branch_context_sha256=sha(branch/'model.blend'),limitations=['Native grass domain retained; hidden leaf depth and reverse appearance inferred from own source.','Final geometry uses rooted blades and independent native pixel fragments; the intermediate leaf hypothesis is replaced, including its foreground constraints.','Archival terrain support is provisional; full bank refinement remains.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(worker)];render_candidate.main()


if __name__=='__main__':main()
