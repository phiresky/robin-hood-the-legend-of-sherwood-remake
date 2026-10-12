"""Single bounded lease for Tree18 branch union, saved views and joint source receivers."""
import json
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_tree18_union_saved as union
import restart2_tree18_saved_review as review
import restart2_tree18_priority_saved_receivers as receivers
import render_slots
from render_slots import acquire, release
import restart2_tree01_full_native_saved_guard as tree01_phases


def main():
    union.guard.resources(); acquire(slots=2)
    originals = []
    pool_release = render_slots.release
    try:
        union.guard.resources()
        for module in (union.guard, review.guard):
            originals.append((module, module.release))
            module.release = lambda: False
        render_slots.release = lambda: False
        phaseout = union.B / 'tree01-bark-source-432-projection-v1/full-native-plane-diagnostic-v3'
        sys.argv = ['resume-tree01-primary-rays', '--', '--model', str(union.B / 'tree01-bark-source-432-projection-v1/worker.blend'), '--output', str(phaseout)]
        tree01_phases.main()
        sys.argv = ['tree18-union-priority-round', '--review']
        union.main()
        receivers.main()
        out = union.B / 'tree18-united-wood-saved-v1'
        proof = union.B / 'tree18-priority-saved-receivers-v1'
        size = sum(p.stat().st_size for folder in (out, proof, phaseout) for p in folder.rglob('*') if p.is_file())
        assert size < 32 * 1024**2
        (out / 'bounded-round.json').write_text(json.dumps({'status': 'EXECUTED_PENDING_PERSONAL_VISUAL_AND_RECEIVER_REVIEW', 'bytes': size, 'model_sha256': union.sha(out / 'worker.blend'), 'limits': ['One shared FIFO lease, sequential2threads. No canonical writes.', 'Receiver result may be HOLD; successful execution does not approve receiver classes or full asset.', 'No source texture assignments;650bark/31stone remain distinct proposals.']}, indent=2) + '\n')
    finally:
        for module, function in originals:
            module.release = function
        render_slots.release = pool_release
        release()


if __name__ == '__main__':
    main()
