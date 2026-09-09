"""Retain the pixel detail of the source instead of bilinear texture blur."""
import bpy

baseline=bpy.data.collections['00 Baseline - original obstacle reconstruction']
protected={m for o in baseline.objects if o.type=='MESH' for m in o.data.materials}
copies={}
changed=set()
scene=bpy.data.scenes['Sherwood Refinement']
for c in scene.collection.children:
    if c.name.startswith(('00 ','07 ')):continue
    for obj in c.objects:
        if obj.type!='MESH' or obj.hide_render:continue
        for slot,material in enumerate(obj.data.materials):
            if material in protected:
                if material not in copies:
                    copies[material]=material.copy()
                    copies[material].name=material.name+' - source pixel sampling'
                material=copies[material]
                obj.data.materials[slot]=material
            if not material.node_tree:continue
            for node in material.node_tree.nodes:
                if node.type=='TEX_IMAGE':
                    node.interpolation='Closest';changed.add(material.name)
result={'materials':len(changed),'baseline_materials_preserved':len(copies)}
