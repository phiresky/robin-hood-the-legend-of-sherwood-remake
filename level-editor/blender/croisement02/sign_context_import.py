"""Import physical context only after evaluating and verifying saved transforms."""
import bpy
import numpy as np


def append_verified(scene,path,names,expected):
    with bpy.data.libraries.load(str(path),link=False) as (source,data):
        data.objects=list(names)
    for obj in data.objects:
        scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    receipts=[]
    for source_name,obj in zip(names,data.objects):
        matrix=obj.matrix_world.copy();reference=np.array(expected[source_name]['matrix_world'])
        drift=float(np.max(abs(np.array(matrix)-reference)))
        if drift>1e-6:
            raise ValueError(f'Evaluated imported transform differs from reopened source: {source_name}: {drift}')
        obj.parent=None;obj.matrix_world=matrix
        obj.hide_render=False
        receipts.append(dict(source_object=source_name,imported_object=obj.name,matrix_world=[list(r) for r in matrix],maximum_source_matrix_error=drift))
    bpy.context.view_layer.update()
    for obj,row in zip(data.objects,receipts):
        if np.max(abs(np.array(obj.matrix_world)-np.array(row['matrix_world'])))>1e-6:
            raise ValueError('World transform drift after deparenting '+obj.name)
    return data.objects,receipts
