"""Blender fixture: authored scenery exports as a visual-only part with no game obstacle."""
from pathlib import Path
import json
import sys
import tempfile
import bpy
sys.path.insert(0,str(Path(__file__).resolve().parent))
from export_editor import export_editor, export_asset_library
from verify_publication_assets import verify, gltf


def check():
    with tempfile.TemporaryDirectory() as directory:
        root=Path(directory)
        bpy.ops.wm.read_factory_settings(use_empty=True)
        collection=bpy.data.collections.new('Fixture Working')
        bpy.context.scene.collection.children.link(collection)
        catalog={'version':2,'map':'Fixture','canonical_owners':{'building-000':'hut','foliage-oak':'oak'},'groups':[
            {'id':'hut','name':'Hut','parts':[{'obstacle':0,'name':'Walls'}]},
            {'id':'oak','name':'Oak','parts':[{'node':'foliage-oak','name':'Painted tree','foliage_domain_mask':7}]}]}
        for name,x,group,part,source in [('hut',0,'Hut','Walls','building-000'),('oak',10,'Oak','Painted tree','foliage-oak')]:
            mesh=bpy.data.meshes.new(name)
            mesh.from_pydata([(x,0,0),(x+2,0,0),(x+2,3,0),(x,3,0),(x,0,4),(x+2,0,4),(x+2,3,4),(x,3,4)],[],[(0,1,2,3),(4,7,6,5),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
            obj=bpy.data.objects.new(name,mesh);collection.objects.link(obj)
            for key,value in {'asset_group':name,'asset_name':group,'part_name':part,'source_node':source}.items():obj[key]=value
            if source.startswith('building-'):obj['source_obstacle']=0
        image=bpy.data.images.new('Fixture / unsafe atlas',width=2,height=2)
        image.pixels[:]=[.2,.4,.1,1.]*4;image.pack()
        material=bpy.data.materials.new('Foliage');material.use_nodes=True
        texture=material.node_tree.nodes.new('ShaderNodeTexImage');texture.image=image
        material.node_tree.links.new(texture.outputs['Color'],material.node_tree.nodes.get('Principled BSDF').inputs['Base Color'])
        bpy.data.objects['oak'].data.materials.append(material)
        bpy.data.objects['oak'].data.uv_layers.new(name='UVMap')
        obstacle={'points':[{'x':x,'y':y,'z_bottom':0,'z_top':100}for x,y in [(-1,-1),(1,-1),(1,1),(-1,1)]],'opaque':True,'solid':True,'mouse':True,'projection_area':[0,0],'show_shadow_polygon':False,'default_material':0,'material_indices':[]}
        level={'sight_obstacles':[obstacle]};level_path=root/'level.json';level_path.write_text(json.dumps(level));catalog_path=root/'catalog.json';catalog_path.write_text(json.dumps(catalog))
        export_asset_library('Fixture',root/'assets',level_path,catalog=catalog)
        report=export_editor('Fixture',root/'fixture.rhlos-map.json',catalog=catalog,level=level)
        assert image.name=='Fixture / unsafe atlas'
        (root/'stage.json').write_text(json.dumps({'map':report,'generated_materials':{}}))
        verify(root,catalog_path)
        entries={entry['id']:entry for entry in json.loads((root/'assets/index.json').read_text())['assets']}
        descriptor=json.loads((root/'assets'/entries['oak']['descriptor']).read_text())
        assert descriptor['parts']==[{'node':'foliage-oak','name':'Painted tree','scenery':True,'default_hidden':False}],descriptor['parts']
        part_node=next(n for n in gltf(root/'assets'/entries['oak']['model'])['nodes'] if n.get('name')=='foliage-oak')
        assert part_node['extras'].get('scenery') is True and 'source_obstacle' not in part_node['extras']
        sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
        from stored_map import expand_document
        document=expand_document(Path(report['library']),json.loads((root/'fixture.rhlos-map.json').read_text()))
        tree=next(o for o in document['objects'] if o['node'].endswith(':foliage-oak'))
        assert 'obstacle' not in tree and tree['kind']=='scenery' and tree['source']=={'map':'Fixture'},tree
        # The descriptor cannot later acquire a fabricated footprint.
        path=root/'assets'/entries['oak']['descriptor'];saved=path.read_text()
        bad=json.loads(saved);bad['parts'][0]['obstacle_local_game']=obstacle;path.write_text(json.dumps(bad))
        try:verify(root,catalog_path)
        except ValueError:pass
        else:raise AssertionError('Accepted a scenery part with a game footprint')
        path.write_text(saved)
        bpy.data.objects['oak']['source_obstacle']=0
        try:export_editor('Fixture',root/'bad.rhlos-map.json',catalog=catalog,level=level)
        except ValueError:pass
        else:raise AssertionError('Accepted a scenery mesh aliasing a sight obstacle')
    print('PASS authored scenery exports without obstacle, footprint or mission profile')


def check_import():
    from supplemental_parts import import_scenery_part
    with tempfile.TemporaryDirectory() as directory:
        root=Path(directory)
        bpy.ops.wm.read_factory_settings(use_empty=True)
        mesh=bpy.data.meshes.new('stage');mesh.from_pydata([(0,0,0),(1,0,0),(0,1,0)],[],[(0,1,2)])
        obj=bpy.data.objects.new('Landing stage',mesh);bpy.context.scene.collection.objects.link(obj)
        for key,value in {'asset_group':'pond','source_node':'scenery-pond-landing-stage'}.items():obj[key]=value
        bpy.ops.wm.save_as_mainfile(filepath=str(root/'handoff.blend'))
        bpy.ops.wm.read_factory_settings(use_empty=True)
        collection=bpy.data.collections.new('Fixture Working');bpy.context.scene.collection.children.link(collection)
        item={'asset_id':'pond','source_nodes':['scenery-pond-landing-stage'],'object_names':['Landing stage'],
              'blend_path':str(root/'handoff.blend')}
        for bad in ({'mission_profile':'Map - Pond'},{'source_nodes':['building-001']}):
            try:import_scenery_part({**item,**bad},collection.name)
            except ValueError:pass
            else:raise AssertionError('Accepted an invalid scenery import: '+repr(bad))
        result=import_scenery_part(item,collection.name)
        assert result['supplemental_scenery_part'] and 'mission_profile' not in result,result
        imported=bpy.data.objects['Landing stage']
        assert imported.users_collection[0]==collection and 'drawbridge_export_mode' not in imported
    print('PASS authored scenery imports as a supplemental part without mission metadata')


if __name__=='__main__':
    check()
    check_import()
