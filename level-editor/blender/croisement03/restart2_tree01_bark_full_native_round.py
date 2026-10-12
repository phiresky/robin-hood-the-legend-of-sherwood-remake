"""One bounded private bark receiver, projection, saved-view and full-alpha diagnostic round."""
import json
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_saved_bark_receivers as receivers
import restart2_tree01_project_bark_seeds as projection
import restart2_tree01_bark_projection_review as review
import restart2_tree01_full_native_saved_guard as phases
import render_slots
B=projection.B
OUT=B/'tree01-bark-source-432-projection-v1'
PROOF=B/'tree01-native-transition-saved-v2/bark-receiver-147-v1'


def main():
    assert not OUT.exists() and not PROOF.exists()
    common=projection.common
    common.resources();render_slots.acquire(slots=2)
    pool_release=render_slots.release;common_release=common.release
    try:
        common.resources()
        render_slots.release=lambda:False
        common.release=lambda:False
        sys.argv=['tree01-bark-round','--','--proposal',str(B/'tree01-bark-completeness-v2/proposed-additional-bark.png'),'--count','147','--out',str(PROOF)]
        receivers.main()
        assert json.loads((PROOF/'receipt.json').read_text())['status']=='PASS_SAVED_BARK_RECEIVERS'
        projection.EXTRA_PROOF=PROOF/'receipt.json';projection.OUT=OUT
        projection.main()
        sys.argv=['tree01-bark-round','--','--folder',str(OUT)]
        review.main()
        sys.argv=['tree01-bark-round','--','--model',str(OUT/'worker.blend'),'--output',str(OUT/'full-native-plane-diagnostic-v1')]
        phases.main()
        size=sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())+sum(p.stat().st_size for p in PROOF.rglob('*')if p.is_file())
        assert size<32*1024**2
        (OUT/'bounded-round.json').write_text(json.dumps({'status':'EXECUTED_PENDING_PERSONAL_VISUAL_AND_PHASE_RESULT_REVIEW','bytes':size,'model_sha256':common.sha(OUT/'worker.blend'),'stages':['saved147receivers','285safe-plus147-source-projection','complete-alpha-UV-material-domain','saved-native-and-eightviews','full14native-plane-diagnostic'],'limits':['Single bounded shared FIFO lease, sequential2threads, no canonical or sourcepixel edits.','Full native plane conflicts are diagnostic and never authorize geometry distortion or removal of native pixels.','No final asset approval implied.']},indent=2)+'\n')
    finally:
        common.release=common_release;render_slots.release=pool_release;pool_release()


if __name__=='__main__':
    main()
