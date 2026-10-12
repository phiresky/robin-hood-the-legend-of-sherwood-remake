"""Diagnostic Cycles source coverage with imported single-sided materials enforced."""
from pathlib import Path
original=Path(__file__).with_name('joined02_coverage_v5.py')
source=original.read_text()
needle=" source=source.replace(needle,needle+\"\\n  for obj in added:\\n   if obj.type=='MESH':obj.is_holdout=True\")"
assert source.count(needle)==1
replacement=''' source=source.replace(needle,needle+"\\n  for obj in added:\\n   if obj.type=='MESH':\\n    obj.is_holdout=True\\n    for slot in obj.material_slots:\\n     mat=slot.material\\n     if mat is None or not mat.use_backface_culling or mat.get('diagnostic_backfaces'):continue\\n     assert mat.use_nodes\\n     nt=mat.node_tree;outputs=[n for n in nt.nodes if n.type=='OUTPUT_MATERIAL' and n.is_active_output]\\n     assert len(outputs)==1\\n     surface=outputs[0].inputs['Surface'];assert len(surface.links)==1\\n     previous=surface.links[0].from_socket;geometry=nt.nodes.new('ShaderNodeNewGeometry');transparent=nt.nodes.new('ShaderNodeBsdfTransparent');mix=nt.nodes.new('ShaderNodeMixShader')\\n     nt.links.new(geometry.outputs['Backfacing'],mix.inputs[0]);nt.links.new(previous,mix.inputs[1]);nt.links.new(transparent.outputs[0],mix.inputs[2]);nt.links.new(mix.outputs[0],surface);mat['diagnostic_backfaces']=True")'''
source=source.replace(needle,replacement)
source=source.replace('joined02-v5-holdout-coverage-256','joined02-v6-holdout-explicit-backfaces')
source=source.replace('Static neighbor meshes use Cycles holdout without modifying topology, transforms or source alpha.', 'Static neighbor meshes use Cycles holdout; explicit transparent backface nodes enforce imported single-sided material flags. Geometry, transforms and original alpha unchanged.')
source=source.replace('available()<14*1024**3','available()<11*1024**3').replace('Waiting14GiB','Waiting11GiB')
exec(compile(source,__file__,'exec'),{'__name__':'__main__','__file__':__file__})
