"""Migrate the frozen legacy Sherwood refinement without changing its geometry.

Run from level-editor with Blender --background --python ... -- --output DIR.
This imports legacy evidence, not a new geometry/texture approval. The old scene
stays untouched. Source gameplay descriptors and map placements remain authoritative.
"""
import argparse, hashlib, json, math, re, struct, sys
from pathlib import Path
import bpy
import numpy as np

EDITOR = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(Path(__file__).parent), str(EDITOR/'refinement/blender'), str(EDITOR/'refinement')]
from editor_catalog import NAMES, PART_NAMES, source_node, asset_type
from export_editor import export_editor
from asset_index import write_asset_index
import render_slots


def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def write(path, value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(value,indent=2)+'\n')


def runtime_material(name, image, uv_name, alpha=False):
    m=bpy.data.materials.new(name);m.use_nodes=True
    ns=m.node_tree.nodes;ns.clear();ls=m.node_tree.links
    uv=ns.new('ShaderNodeUVMap');uv.uv_map=uv_name
    tex=ns.new('ShaderNodeTexImage');tex.image=image;tex.extension='EXTEND';tex.interpolation='Closest'
    shader=ns.new('ShaderNodeBsdfPrincipled');shader.inputs['Roughness'].default_value=1
    out=ns.new('ShaderNodeOutputMaterial')
    ls.new(uv.outputs['UV'],tex.inputs['Vector']);ls.new(tex.outputs['Color'],shader.inputs['Base Color'])
    if alpha:ls.new(tex.outputs['Alpha'],shader.inputs['Alpha']);m.surface_render_method='DITHERED'
    ls.new(shader.outputs[0],out.inputs['Surface'])
    return m


LEAVES={}
def static_leaves(obj):
    """Use the same first-frame cutout convention as the published Sherwood palette."""
    if 'Source canvas' not in obj.data.uv_layers:return
    source=obj.data.materials[0]
    tex=next(n for n in source.node_tree.nodes if n.type=='TEX_IMAGE')
    transform=tex.inputs['Vector'].links[0].from_node
    scale=transform.inputs[0].links[0].from_node.inputs[1].default_value
    offset=transform.inputs[1].default_value
    atlas=tex.image;aw,ah=atlas.size
    w,h=round(scale[0]*aw),round(scale[1]*ah);x,y=round(offset[0]*aw),round(offset[1]*ah)
    if source.name not in LEAVES:
        pixels=np.empty(aw*ah*4,dtype=np.float32);atlas.pixels.foreach_get(pixels)
        tile=np.zeros((h+2,w+2,4),dtype=np.float32);tile[1:-1,1:-1]=pixels.reshape(ah,aw,4)[y:y+h,x:x+w]
        image=bpy.data.images.new(source.name+' first frame',width=w+2,height=h+2,alpha=True)
        image.pixels.foreach_set(tile.ravel());image.pack()
        LEAVES[source.name]=runtime_material(source.name+' runtime leaves',image,'Static leaves',True)
    uv=obj.data.uv_layers.new(name='Static leaves');original=obj.data.uv_layers['Source canvas']
    for i,loop in enumerate(original.data):uv.data[i].uv=((loop.uv.x*w+1)/(w+2),(loop.uv.y*h+1)/(h+2))
    for i in range(len(obj.data.materials)):obj.data.materials[i]=LEAVES[source.name]


def bake_bark(objects, output):
    """Bake Blender's vertex-weighted source/tiled bark blend into portable UVs."""
    scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=1
    scene.cycles.use_denoising=False
    by_source={}
    for obj in objects:
        if any(m and m.name.startswith('Bark - synthesized') for m in obj.data.materials):
            by_source.setdefault(obj['source_node'],[]).append(obj)
    report=[]
    for source, obs in sorted(by_source.items()):
        bpy.ops.object.select_all(action='DESELECT')
        for obj in obs:obj.select_set(True)
        bpy.context.view_layer.objects.active=obs[0]
        vertices=sum(len(o.data.vertices) for o in obs);faces=sum(len(o.data.polygons) for o in obs)
        bpy.ops.object.join();obj=bpy.context.object
        assert len(obj.data.vertices)==vertices and len(obj.data.polygons)==faces
        # Freeze the original implicit UV input before adding a new active layer.
        original_uv=obj.data.uv_layers.active.name
        for i,mat in enumerate(list(obj.data.materials)):
            mat=mat.copy();obj.data.materials[i]=mat
            for tex in [n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and not n.inputs['Vector'].is_linked]:
                uv=mat.node_tree.nodes.new('ShaderNodeUVMap');uv.uv_map=original_uv
                mat.node_tree.links.new(uv.outputs['UV'],tex.inputs['Vector'])
        obj.data.uv_layers.new(name='Runtime bark')
        obj.data.uv_layers.active_index=len(obj.data.uv_layers)-1
        bpy.ops.object.mode_set(mode='EDIT');bpy.ops.mesh.select_all(action='SELECT')
        bpy.ops.uv.smart_project(angle_limit=math.radians(66),island_margin=.008)
        bpy.ops.object.mode_set(mode='OBJECT')
        size=2048 if vertices>1000 else 1024
        target=bpy.data.images.new(source+' baked bark',width=size,height=size,alpha=False)
        for mat in obj.data.materials:
            n=mat.node_tree.nodes.new('ShaderNodeTexImage');n.image=target;mat.node_tree.nodes.active=n
        bpy.ops.object.bake(type='EMIT',margin=8,margin_type='EXTEND',use_clear=True,uv_layer='Runtime bark')
        target.filepath_raw=str(output/(source+'-bark.png'));target.file_format='PNG';target.save();target.pack()
        obj.data.materials.clear();obj.data.materials.append(runtime_material(source+' runtime bark',target,'Runtime bark'))
        for face in obj.data.polygons:face.material_index=0
        report.append(dict(source_node=source,vertices=vertices,faces=faces,texture_sha256=sha(target.filepath_raw)))
        print('BAKED',source,flush=True)
    return report


def finalize_glb(path, descriptor):
    data=path.read_bytes();length,kind=struct.unpack_from('<II',data,12);doc=json.loads(data[20:20+length])
    # Restore native part nodes swallowed by a legacy trunk replacement; partition below.
    present={n.get('name') for n in doc['nodes']}
    parent=next(i for i,n in enumerate(doc['nodes']) if n.get('extras',{}).get('asset_group')==descriptor['id']) if descriptor['parts'] else None
    for part in descriptor['parts']:
        if part['node'] not in present:
            if part['node']!='building-048':raise ValueError('Missing unexpected part: '+part['node'])
            doc['nodes'][parent].setdefault('children',[]).append(len(doc['nodes']))
            doc['nodes'].append({'name':part['node'],'extras':{'source_node':part['node'],'source_obstacle':48,'part_name':part['name']}})
    for m in doc.get('materials',[]):
        m.setdefault('extensions',{})['KHR_materials_unlit']={}
        if 'runtime leaves' in m.get('name',''):
            m.update(alphaMode='MASK',alphaCutoff=.35,doubleSided=True)
        elif 'emissiveTexture' in m:
            m['pbrMetallicRoughness']={'baseColorTexture':m.pop('emissiveTexture')};m.pop('emissiveFactor',None)
    doc['extensionsUsed']=sorted(set(doc.get('extensionsUsed',[])+['KHR_materials_unlit']))
    chunk=json.dumps(doc,separators=(',',':')).encode();chunk+=b' '*(-len(chunk)%4);binary=data[20+length:]
    path.write_bytes(struct.pack('<4sII',b'glTF',2,20+len(chunk)+len(binary))+struct.pack('<II',len(chunk),kind)+chunk+binary)
    from partition_oak import partition
    partition(path, descriptor)


def main(output):
    output=Path(output).resolve();output.mkdir(parents=True,exist_ok=False)
    source=EDITOR/'work/sherwood-refinement/sherwood-refinement.blend'
    source_sha=sha(source)
    library=EDITOR/'library';document=json.loads((library/'scenes/sherwood.rhlos-map.json').read_text())
    descriptors={};owners={};before={};renames={}
    for ref in document['assetSources']+document['sceneAssets']:
        path=library/ref['descriptor'];d=json.loads(path.read_text())
        old_id=d['id']
        if old_id.startswith('sherwood-group-'):
            d['name']=NAMES[int(old_id.rsplit('-',1)[-1])]
            d['id']='sherwood-'+re.sub(r'[^a-z0-9]+','-',d['name'].lower()).strip('-')
        renames[old_id]=d['id'];ref['id']=d['id'];descriptors[d['id']]=d
        before[ref['descriptor']]=sha(path);before[ref['model']]=sha(library/ref['model'])
        for part in d['parts']:owners[part['node']]=d['id']
    for placement in document['placements']:
        placement['assets']=[renames[a] for a in placement['assets']]
        placement['id']=placement['assets'][0]
    before['scenes/sherwood.rhlos-map.json']=sha(library/'scenes/sherwood.rhlos-map.json')
    render_slots.acquire();bpy.ops.wm.open_mainfile(filepath=str(source))
    original=bpy.data.scenes['Sherwood Refinement'];bpy.context.window.scene=original;original.frame_set(1)
    selected=[]
    for c in original.collection.children:
        if c.hide_render or c.name.startswith(('07','11')):continue
        for obj in c.all_objects:
            if obj.type=='MESH' and not obj.hide_render:
                node=source_node(c.name,{'name':obj.name,'props':{k:str(obj[k]) for k in obj.keys()}})
                selected.append((obj,node))
    scene=bpy.data.scenes.new('Sherwood Editor Migration');bpy.context.window.scene=scene
    working=bpy.data.collections.new('Sherwood Working');scene.collection.children.link(working)
    scenery_id='sherwood-foreground-oak'
    owners.update({'ground':'sherwood-terrain','foliage-foreground-oak':scenery_id})
    descriptors[scenery_id]={'version':1,'kind':'projection-mapped-asset','id':scenery_id,'name':'Foreground oak canopy','source_map':'Sherwood','model':'model.glb','model_scene':'Sherwood Editor Export','source_origin_scene':[445,-1120/math.sin(math.radians(35)),0],'parts':[{'node':'foliage-foreground-oak','name':'Foreground oak canopy','scenery':True}]}
    records=[]
    for src,node in selected:
        asset=owners[node];d=descriptors[asset]
        label=NAMES[int(asset.rsplit('-',1)[-1])] if 'sherwood-group-' in asset else d['name']
        world=src.matrix_world.copy()
        obj=src.copy();obj.data=src.data.copy();working.objects.link(obj);obj.hide_set(False);obj.hide_viewport=False
        # The imported glTF coordinate conversion can live on a comparison
        # scene parent. Keep the reusable worker independent of that parent.
        obj.parent=None;obj.matrix_world=world
        obj['source_node']=node;obj['asset_group']=asset;obj['asset_name']=label
        obj['part_name']=PART_NAMES.get(int(node[9:]),label) if node.startswith('building-') else label
        if node.startswith('building-'):obj['source_obstacle']=int(node[9:])
        elif 'source_obstacle' in obj:del obj['source_obstacle']
        static_leaves(obj)
        records.append(dict(object=src.name,source_node=node,asset_id=asset,vertices=len(src.data.vertices),faces=len(src.data.polygons)))
    write(output/'ownership.json',records)
    bakes=output/'bakes';bakes.mkdir();bark=bake_bark(list(working.objects),bakes)
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'editor-migration.blend'))
    root=output/'map-assets/3d-assets';refs={r['id']:r for r in document['assetSources']+document['sceneAssets']}
    for asset,d in descriptors.items():
        label=NAMES[int(asset.rsplit('-',1)[-1])] if 'sherwood-group-' in asset else d['name']
        d['name']=label;d['asset_type']=asset_type(label)
        for part in d['parts']:
            n=part.get('source_obstacle');part['name']=PART_NAMES.get(n,NAMES.get(n,label))
        pivot=d.get('source_origin_scene',[0,0,0])
        directory=root/'sherwood'/asset
        report=export_editor('Sherwood',directory/'model.glb',asset_id=asset,standalone_pivot=pivot)
        for key in ('resources','lossy_model','preview_model'):d.pop(key,None)
        d['resources']=[]
        d['model']='model.glb';d['model_scene']='Sherwood Editor Export'
        d['legacy_refinement']={'source_blend_sha256':source_sha,'geometry':'legacy reconstruction; no new geometry pass',
            'foliage':'static first authored frame, matching the existing named palette',
            'limitations':'Inherited hidden-surface inference and unfinished background props; not re-reviewed under the newer procedure.'}
        finalize_glb(directory/'model.glb',d);write(directory/'asset.json',d)
        ref={'id':asset,'model':f'3d-assets/sherwood/{asset}/model.glb','model_sha256':sha(directory/'model.glb'),
             'descriptor':f'3d-assets/sherwood/{asset}/asset.json','descriptor_sha256':sha(directory/'asset.json')}
        if asset in refs:refs[asset].update(ref)
        else:
            document['assetSources'].append(ref)
            document['placements'].append({'id':asset,'transform':{'dx':445,'dy':1120,'dz':0,'rot_deg':0},'assets':[asset]})
    write_asset_index(root)
    write(output/'sherwood.rhlos-map.json',document)
    write(output/'migration.json',{'version':1,'source_blend':str(source),'source_sha256':source_sha,'recipe_sha256':sha(__file__),'catalog_sha256':sha(EDITOR/'refinement/catalogs/sherwood.json'),
          'before':before,'renames':renames,'objects':len(records),'bark_bakes':bark,'assets':len(descriptors),
          'export_partition_parts':['building-048'],'source_document_placement_transforms_preserved':True})
    print('STAGED',str(output),flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--output',required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);main(args.output)
    from finalize_editor_materials import main as finalize_materials
    from verify_editor_migration import verify
    from verify_editor_handoff import main as verify_handoff
    finalize_materials(args.output)
    verify(args.output)
    verify_handoff(args.output)
    render_slots.release()
    from lossy_assets import refresh_derivatives
    out=Path(args.output)
    render_slots.acquire()
    previews=refresh_derivatives(out/'map-assets/3d-assets',out/'previews')
    write(out/'preview-verification.json',previews)
    print('EXPORT AND HANDOFF CHECKS COMPLETE',flush=True)
