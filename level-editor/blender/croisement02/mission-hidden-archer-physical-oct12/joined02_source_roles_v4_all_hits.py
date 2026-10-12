"""Retain all qualified Tree04 intersections to diagnose renderer culling differences."""
from pathlib import Path
original=Path(__file__).with_name('joined02_source_roles_v1.py')
source=original.read_text().replace("joined02-source-roles-v1'","joined02-source-roles-v4-all-hits'")
source=source.replace("textures[(placed['id'],ni,pi)]=texture_record(mat)",'return')
source=source.replace('            if hit:\n','            if False:\n')
source=source.replace('first_tree_hit=hit))',"first_tree_hit=hit,all_tree_hits=ray['hits']))")
source=source.replace("exact_base_samples=sum(r['tree_nearest_rgba']==r['base_rgba']for r in rows)","rgb_sampling='not performed; all intersections retained'")
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
