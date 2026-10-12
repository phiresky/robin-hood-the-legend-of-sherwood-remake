"""Prepare scoped fallback inputs for the three additional audited trees."""
import importlib.util,json,sys
from pathlib import Path
p=Path(__file__).with_name('prepare_tree39_gray_fill.py');spec=importlib.util.spec_from_file_location('prep',p);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
n=int(sys.argv[sys.argv.index('--')+1]);assert n in (18,19,25)
e=json.loads((m.B/('restart8-five-bark-approved-export-v1' if n==18 else 'restart25-approved-appearance-delta-v1')/f'tree-{n}-v1/export.json').read_text())
rows=json.loads((m.B/'tree-gray-material-audit-oct12-v1/remaining-provenance.json').read_text())['assets'];row=next(x for x in rows if x['tree']==n)
m.A=f'croisement02-tree-{n}';m.LIVE=m.R/'level-editor/library/3d-assets/croisement02'/m.A/'model.glb';m.OUT=m.B/f'tree{n}-gray-fill-input-oct12-v1';m.EXPECTED=row['model_sha256'];m.PIVOT=e['pivot'];m.GRAY_COUNT=1
m.CLOSE=m.B/(f'restart6-five-bark-fill-v1/tree-{n}-fill-v1/shader-restored-v3/close-views.json' if n==18 else f'restart8-toe-bark-fill-v1/tree-{n}-fill-v1/unseen-complete-v3/close-views.json')
m.main()
