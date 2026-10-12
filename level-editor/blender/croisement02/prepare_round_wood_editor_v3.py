"""Fresh current-stage normal editor proof, retaining all six behavior checks."""
import importlib.util,json
from pathlib import Path
p=Path(__file__).with_name('prepare_round_wood_editor_check.py');spec=importlib.util.spec_from_file_location('prep',p);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
m.STAGE=m.BASE/'publication/stage-v3-current';m.OUT=m.BASE/'editor-check-v3';m.main()
# Exact duplicate-title UI row selection, already diagnosed and validated in reload continuation.
source=m.BASE/'editor-reload-v1/reload-check.js';(m.OUT/'reload-check.js').write_text(source.read_text())
r=json.loads((m.OUT/'browser-request.json').read_text());r.update(fixture_correction={'source':str(source),'sha256':m.sha(source),'behavior':'Map document group index identifies actual unfiltered UI row; actual click then exact selected group ID checked for both insertions.'},status='FRESH_V3_READY_STATE_CHECK_EXTENSION_PENDING',previous_proof_not_relabelled=True)
m.dump(m.OUT/'browser-request.json',r)
