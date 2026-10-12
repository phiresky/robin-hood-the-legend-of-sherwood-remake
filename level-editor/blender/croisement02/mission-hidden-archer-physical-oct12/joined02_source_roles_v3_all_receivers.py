"""Classify all foreground receiver identities at sampled missing source pixels."""
from pathlib import Path
original=Path(__file__).with_name('joined02_source_roles_v1.py')
source=original.read_text().replace("joined02-source-roles-v1'","joined02-source-roles-v3-all-receivers'")
# This pass needs foreground identities only. Do not retain texture arrays from
# multiple receivers; the earlier Tree04 pass already records RGB evidence.
source=source.replace("textures[(placed['id'],ni,pi)]=texture_record(mat)", 'return')
source=source.replace('            if hit:\n', '            if False:  # No retained cross-receiver RGB cache.\n')
source=source.replace("asset_ids={'croisement02-tree-04'}",'asset_ids=None')
source=source.replace("exact_base_samples=sum(r['tree_nearest_rgba']==r['base_rgba']for r in rows)",
                      "rgb_sampling='not performed; receiver identities only'")
source=source.replace("tree_nearest_rgba", "receiver_nearest_rgba").replace("first_tree_hit", "first_receiver_hit")
source=source.replace('tree_hits=', 'receiver_hits=')
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
