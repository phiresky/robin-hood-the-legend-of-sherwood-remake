"""Blender fixture for isolated mesh replacement and retained scene state."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import tempfile
import bpy
from import_reviewed_geometry import import_asset_geometry

collection=bpy.data.collections.new('Fixture Working')
bpy.context.scene.collection.children.link(collection)
root=bpy.data.objects.new('Logical root',None)
root['asset_group']='fixture';root.location=(11,23,7)
collection.objects.link(root)
def mesh(name,node,owner='fixture'):
    data=bpy.data.meshes.new(name)
    data.from_pydata([(0,0,0),(1,0,0),(0,1,0)],[],[(0,1,2)])
    obj=bpy.data.objects.new(name,data);obj.parent=root
    obj['source_node']=node;obj['asset_group']=owner
    collection.objects.link(obj)
    return obj
first=mesh('First','building-001')
extra=mesh('Added component','building-001')
hidden=mesh('Retained original','building-001');hidden.hide_render=True
outside=mesh('Other asset','building-002','outside')
bpy.context.view_layer.update()
world=first.matrix_world.copy()
out=Path(tempfile.mkdtemp(prefix='geometry-handoff-'))/'worker.blend'
bpy.ops.wm.save_as_mainfile(filepath=str(out),copy=True)
bpy.data.objects.remove(extra,do_unlink=True)
first.data.vertices[1].co.x=4
old=mesh('Deleted old component','building-001')
before_names=set(bpy.data.objects.keys())
report=import_asset_geometry(out,asset_id='fixture',object_names=['First','Added component'],collection_name=collection.name)
assert bpy.data.objects['First'].data.vertices[1].co.x==1
assert bpy.data.objects['First'].matrix_world==world
assert bpy.data.objects['First'].parent==root
assert bpy.data.objects.get('Deleted old component') is None
assert report['hidden_originals_preserved']==1
assert set(bpy.data.objects.keys())==(before_names-{'First','Deleted old component'})|{'First','Added component'}
print('PASS: corrected geometry, added/deleted components, transforms, hidden originals and outside state')
