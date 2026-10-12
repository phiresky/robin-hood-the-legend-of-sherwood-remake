"""Targeted native-camera state appearance comparison on the implicated crown only."""
from pathlib import Path
base = Path(__file__).with_name('joined02_receiver_rgb_v1.py')
source = base.read_text()
source = source.replace("source=Path(original.__file__).read_text();needle=", "source=Path(original.__file__).read_text().replace(\"for candidate in row['bounds_candidates']:\", \"for candidate in [c for c in row['bounds_candidates'] if c['id']=='croisement02-tree-04']:\");needle=")
source = source.replace('joined02-receiver-rgb-diagnostic-v1','joined02-receiver-rgb-diagnostic-v2-target')
source = source.replace('available()<14*1024**3','available()<11*1024**3').replace('Waiting14GiB','Waiting11GiB')
needle = "    report['targets']="
assert source.count(needle) == 1
source = source.replace(needle, "    report['source_receiver_candidates']=[c for c in report['source_receiver_candidates'] if c['id']=='croisement02-tree-04']\n    report['limits']+=['Targeted Tree04-only source-camera comparison. Other static receivers are represented only by the source backplate; this is not a full joined-scene clearance proof.']\n"+needle)
exec(compile(source, str(base), 'exec'), {'__name__':'__main__','__file__':__file__})
