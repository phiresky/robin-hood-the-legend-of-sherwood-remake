"""One bounded FIFO round for reviewed bank source projection and saved context checks."""
import json
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_bank_face_source_saved as projection
import restart2_bank_source_saved_review as review
import restart2_bank_native_foreground_review as native
import restart2_bank_lofts_contact_audit as contact
from render_slots import acquire, release
B=projection.B
OUT=B/'bank-coupled-left-full-source-v1'


def main():
    assert not OUT.exists()
    projection.guard.resources()
    acquire(slots=2)
    originals=[]
    try:
        projection.guard.resources()
        # Child stages are sequential parts of this single bounded lease. Their
        # normal standalone acquisition sees the already-held process lease.
        for module in (projection.guard, native.guard, contact):
            originals.append((module,module.release))
            module.release=lambda: False
        sys.argv=['bank-full-source-round','--','--input',str(B/'bank-coupled-left-saved-v1'),'--out',str(OUT),'--extra-proposal',str(B/'bank-left-exposed-stone-proposal-v1/proposal.json'),'--retained-projection',str(B/'bank-main-masses-source-v1/projection.json'),'--unresolved-receivers',str(B/'bank53-source-retention-v1/receipt.json'),'--full-source-proposal',str(B/'bank-full-visible-stone-audit-v3'),'--residual-source-selection',str(B/'bank-residual-approved-selection-v1')]
        projection.main()
        sys.argv=['bank-full-source-round','--','--folder',str(OUT),'--exact-coupled-traces']
        review.main()
        sys.argv=['bank-full-source-round','--','--folder',str(OUT)]
        contact.main()
        native.main()
        size=sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())
        assert size < 32*1024**2
        (OUT/'bounded-round.json').write_text(json.dumps({'status':'EXECUTED_PENDING_PERSONAL_VISUAL_REVIEW','model_sha256':projection.sha(OUT/'worker.blend'),'bytes':size,'stages':['reviewed-source-projection','exact1834saved-traces','all-source-RGBA','actual-and-solid-eight','independent-solid-contact','full-native-foreground-context'],'limits':['One shared FIFO lease, sequential2thread stages; no canonical writes.','Native card comparison is a geometric diagnostic, not runtime display-priority proof.','User approval and final source ownership remain pending.']},indent=2)+'\n')
    finally:
        for module,function in originals:
            module.release=function
        release()


if __name__=='__main__':
    main()
