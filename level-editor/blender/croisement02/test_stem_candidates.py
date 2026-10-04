"""Selection rejects stale geometry, changed ownership, and inherited approval."""
import json
from pathlib import Path
import tempfile
import unittest

from stem_candidates import ASSET, selected_workspace, sha


class StemSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name)
        self.worker = self.out / 'worker'
        self.worker.mkdir()
        (self.worker / 'model.blend').write_bytes(b'unit-test model identity')
        model = sha(self.worker / 'model.blend')
        actual = self.worker / 'inspection/actual-materials'
        actual.mkdir(parents=True)
        (actual / 'sheet.png').write_bytes(b'unit-test image identity')
        sheet = sha(actual / 'sheet.png')
        records = {'validation.json': {'status': 'PASS'},
            'inspection/saved-model-audit.json': {'status': 'PASS', 'model_sha256': model},
            'inspection/source-coverage/report.json': {'model_sha256': model, 'intersection_over_union': .99},
            'inspection/local-depth.json': {'model_sha256': model, 'rows': [
                {'source_y': y, 'depth_width_ratio': 1.} for y in (1120, 1130, 1140, 1150)]},
            'inspection/actual-materials/evidence.json': {'model_sha256': model, 'sheet_sha256': sheet},
            'inspection/visual-review.json': {'model_sha256': model, 'sheet_sha256': sheet, 'ready_for_geometry_review': True}}
        for name, value in records.items():
            path = self.worker / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value))
        self.catalog = self.out / 'catalog.json'
        self.catalog.write_text(json.dumps({'groups': [{'id': ASSET, 'parts': [{'node': 'foliage-wood-044'}]}]}))
        self.receipt = self.out / 'stem-cleanup-selections/stem-44.json'
        self.receipt.parent.mkdir()
        self.record = dict(asset_id=ASSET, worker=str(self.worker), model_sha256=model,
            part_ids=['foliage-wood-044'], approval='pending',
            evidence_sha256={str(p): sha(p) for p in self.worker.rglob('*') if p.is_file()})
        self.receipt.write_text(json.dumps(self.record))

    def test_current_candidate_selects(self):
        self.assertEqual(selected_workspace(self.out, ASSET, self.catalog), self.worker)

    def test_changed_model_is_rejected(self):
        (self.worker / 'model.blend').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'evidence changed'):
            selected_workspace(self.out, ASSET, self.catalog)

    def test_changed_source_scope_is_rejected(self):
        self.catalog.write_text(json.dumps({'groups': [{'id': ASSET, 'parts': [{'node': 'foliage-wood-009'}]}]}))
        with self.assertRaisesRegex(ValueError, 'ownership changed'):
            selected_workspace(self.out, ASSET, self.catalog)

    def test_inherited_approval_is_rejected(self):
        self.record['approval'] = 'approved'
        self.receipt.write_text(json.dumps(self.record))
        with self.assertRaisesRegex(ValueError, 'Invalid stem selection'):
            selected_workspace(self.out, ASSET, self.catalog)


if __name__ == '__main__':
    unittest.main()
