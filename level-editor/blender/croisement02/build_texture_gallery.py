"""Collect manually reviewed Croisement02 texture experiments without moving them."""
import sys
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from catalog import OUT
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
from build_texture_gallery import collect
from review_evidence import sha
from build_review_gallery import build


def attach_aligned_comparisons(manifest):
    """Keep supplementary before/fill references outside decision fingerprints."""
    data = json.loads(manifest.read_text())
    added = 0
    for item in data["items"]:
        review_path = Path(item["review"])
        review = json.loads(review_path.read_text())
        experiment = review_path.parent
        comparison = experiment / review["bake"] / "baseline-comparison"
        report_path = comparison / "comparison.json"
        if not report_path.exists():
            continue
        report = json.loads(report_path.read_text())
        if (report["baked_model_sha256"] != review["baked_model_sha256"]
                or report["camera_manifest_sha256"] != sha(experiment / "views.json")
                or report["approved_model_sha256"] != sha(experiment / "approved-model.blend")):
            raise ValueError("Aligned comparison no longer matches texture candidate")
        for start in (0, 4):
            name = f"comparison-{start}-{start+3}.png"
            path = comparison / name
            if sha(path) != report["artifacts"][name]:
                raise ValueError("Aligned material comparison changed")
            identifier = f"original-materials-aligned-{start}-{start+3}"
            refs = item.setdefault("artwork_references", [])
            refs[:] = [ref for ref in refs if ref["id"] != identifier]
            refs.append(dict(id=identifier, path=str(path), sha256=sha(path),
                label=f"Views {start}–{start+3}: original approved materials (top), new fill (bottom), identical cameras. Existing card geometry remains unchanged.",
                comparison_report=str(report_path), comparison_report_sha256=sha(report_path)))
            added += 1
    if added:
        manifest.write_text(json.dumps(data, indent=2) + "\n")
    return added


def main():
    roots = {review.parent.parent for round_root in OUT.glob('texture-fill-round-*')
             for review in round_root.rglob('texture-review.json')}
    additional = OUT / 'texture-review/additional-experiment-roots.json'
    if additional.exists():
        for name in json.loads(additional.read_text())['roots']:
            root = Path(name).resolve(strict=True)
            if not root.is_relative_to(OUT.resolve()):
                raise ValueError('Additional texture experiments must stay inside this map workspace')
            roots.add(root)
    roots = sorted(roots)
    if not roots:
        raise ValueError('No manually reviewed texture experiments exist')
    supersessions_path = OUT / 'texture-review/supersessions.json'
    supersessions = json.loads(supersessions_path.read_text())['records'] if supersessions_path.exists() else []
    result = collect(roots[0], OUT / 'texture-review', 'Croisement02', roots[1:], supersessions=supersessions)
    manifest = OUT / 'texture-review/texture-candidates.json'
    if attach_aligned_comparisons(manifest):
        build(manifest, OUT / 'texture-review/gallery', map_name='Croisement02 texture', pending_only=True)
    print(result)


if __name__ == '__main__':
    main()
