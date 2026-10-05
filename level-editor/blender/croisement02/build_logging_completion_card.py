"""Freeze the reviewed logging geometry card for the next grouped handoff."""
import json,sys
from pathlib import Path
from catalog import OUT,reviewed_catalog
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import write_json,sha
from prop_completion_candidates import selected_workspace
from build_review_gallery import build

def main():
    asset='croisement02-logging-clearing-log';w=selected_workspace(OUT,asset,reviewed_catalog())
    if w is None:raise ValueError('Logging completion not selected')
    dest=OUT/'restart3-review-batches/logging-ready-v1'
    if dest.exists():raise FileExistsError(dest)
    dest.mkdir()
    i=w/'inspection';root=json.loads((i/'root-completion-review.json').read_text());joint=json.loads((i/'joint-neighbourhood.json').read_text())
    item=dict(id=asset,name='Logging clearing fallen log and root tangle',status='ready-for-user',technical_eligible=True,
        model=str(w/'model.blend'),solid=str(w/'modified/solid.png'),textured=str(w/'modified/textured.png'),context=str(w/'modified/context.png'),
        stored_material_textured=str(i/'actual-materials/sheet.png'),stored_material_audit=str(i/'saved-model-audit.json'),
        source_comparison=str(i/'source-comparison/comparison.png'),source_comparison_label='Original source, saved geometry and overlay',
        source_comparison_secondary=joint['sheet'],source_comparison_secondary_label=joint['label'],
        source_trace=str(Path(joint['sheet']).parent/'source-overlay.png'),source_trace_label='Native neighborhood source overlay',
        validation=str(w/'validation.json'),review=str(i/'root-completion-review.json'),disclosure=root['physical_source_guards'],notes=root['limitations'])
    index=dest/'review-candidates.json';write_json(index,dict(title='Croisement02 logging root geometry',items=[item]))
    build(index,dest/'gallery',pending_only=True,map_name='Croisement02')
    write_json(dest/'root-selection-binding.json',dict(model_sha256=sha(w/'model.blend'),strict_selection=str(OUT/'restart2-vegetation/prop-selections'/f'{asset}.json'),selection_sha256=sha(OUT/'restart2-vegetation/prop-selections'/f'{asset}.json'),scope='Next grouped batch only; no new user approval'))
if __name__=='__main__':main()
