"""Approval staging must fail closed when previously reviewed evidence changes."""
import json
from pathlib import Path
import tempfile
import shutil
import unittest
from approved_texture_stage import FLAGS, select
from evidence_io import sha


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.base = self.root / 'base.blend'
        self.base.write_bytes(b'base geometry')
        paths = {key: self.root / filename for key, filename in
                 [('model', 'worker.blend'), ('textured', 'textured.png'),
                  ('validation', 'validation.json'), ('review', 'review.json')]}
        paths['model'].write_bytes(b'baked geometry')
        paths['textured'].write_bytes(b'actual rendered sheet')
        paths['validation'].write_text(json.dumps({'source_mask_evidence': {}}))
        paths['review'].write_text(json.dumps(dict(status='ready-for-user',
            all_eight_actual_views_inspected=True, baked_model_sha256=sha(paths['model']),
            actual_sheet_sha256=sha(paths['textured']))))
        proof = dict(asset_id='tree', status='PASS', reopened_preservation='PASS',
            model_sha256=sha(self.base), candidate_model_sha256=sha(paths['model']),
            bake_validation_sha256=sha(paths['validation']), evidence_sha256={}, receiver_names=['Tree'])
        proof.update({key: True for key in FLAGS})
        (self.root / 'reopened-preservation.json').write_text(json.dumps(proof))
        self.decision = dict(asset_id='tree', scope='texture', decision='approved',
            evidence_paths={key: str(path) for key, path in paths.items()},
            evidence_sha256={key: sha(path) for key, path in paths.items()})
        archive = self.root / 'archive'
        archive.mkdir()
        self.decision['archive'] = str(archive)
        (archive / 'decision.json').write_text(json.dumps(self.decision))
        for key, path in paths.items():
            shutil.copy2(path, archive / (key + path.suffix))
        self.decisions = self.root / 'decisions.json'
        self.write_decisions([self.decision])

    def write_decisions(self, records):
        self.decisions.write_text(json.dumps({'decisions': records}))

    def test_valid_and_changed_base(self):
        self.assertEqual(set(select(self.decisions, {'tree': self.base})), {'tree'})
        self.base.write_bytes(b'new geometry')
        with self.assertRaisesRegex(ValueError, 'current geometry'):
            select(self.decisions, {'tree': self.base})

    def test_changed_evidence(self):
        Path(self.decision['evidence_paths']['textured']).write_bytes(b'different render')
        with self.assertRaisesRegex(ValueError, 'Stale approved'):
            select(self.decisions, {'tree': self.base})

    def test_changed_archived_evidence(self):
        (self.root / 'archive' / 'model.blend').write_bytes(b'changed archive')
        with self.assertRaisesRegex(ValueError, 'Archived evidence changed'):
            select(self.decisions, {'tree': self.base})

    def test_failed_preservation(self):
        path = self.root / 'reopened-preservation.json'
        proof = json.loads(path.read_text())
        proof['physical_alpha_unchanged'] = False
        path.write_text(json.dumps(proof))
        with self.assertRaisesRegex(ValueError, 'Incomplete preservation'):
            select(self.decisions, {'tree': self.base})

    def test_latest_rejection_supersedes_approval(self):
        self.write_decisions([self.decision, dict(asset_id='tree', scope='texture', decision='rejected')])
        with self.assertRaisesRegex(ValueError, 'No approved'):
            select(self.decisions, {'tree': self.base})


if __name__ == '__main__':
    unittest.main()
