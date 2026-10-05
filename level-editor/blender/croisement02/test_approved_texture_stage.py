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


class CanopySelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.base = self.root / 'base.blend'
        self.base.write_bytes(b'geometry')
        self.candidate = self.root / 'bake-v1'
        self.candidate.mkdir()
        self.model = self.candidate / 'worker.blend'
        self.model.write_bytes(b'approved texture')
        (self.candidate / 'actual').mkdir()
        self.sheet = self.candidate / 'actual/textured.png'
        self.sheet.write_bytes(b'eight views')
        validation = self.candidate / 'validation.json'
        validation.write_text(json.dumps({'source_mask_evidence': {}}))
        self.proof = self.candidate / 'reopened-preservation.json'
        self.proof.write_text(json.dumps(dict(asset_id='tree', status='PASS',
            reopened_preservation='PASS', model_sha256=sha(self.base),
            candidate_model_sha256=sha(self.model), receiver_names=['Tree'],
            evidence_sha256={}, bake_validation_sha256=sha(validation), **{k: True for k in FLAGS})))
        self.review = self.candidate / 'agent-material-review.json'
        self.review.write_text(json.dumps(dict(ready_for_coordinator_review=True,
            all_eight_saved_model_views_inspected=True, model_sha256=sha(self.model),
            actual_sheet_sha256=sha(self.sheet), reopened_preservation_sha256=sha(self.proof))))
        self.archive = self.root / 'archive'
        self.archive.mkdir()
        (self.archive / 'evidence.json').write_bytes(b'frozen gallery')
        files = [self.model, self.sheet, self.review]
        for p in files:
            shutil.copy2(p, self.archive / p.name)
        row = dict(asset_id='tree', scope='texture', decision='approved', candidate=str(self.candidate),
            model_sha256=sha(self.model), evidence_sha256={str(p): sha(p) for p in files},
            archived_evidence={str(p): str(self.archive / p.name) for p in files},
            gallery_evidence_sha256=sha(self.archive / 'evidence.json'),
            geometry_approval=dict(scope='geometry', decision='approved', model_sha256=sha(self.base)))
        document = dict(snapshot=str(self.archive), decisions=[row])
        self.decisions = self.root / 'decisions.json'
        self.decisions.write_text(json.dumps(document))
        shutil.copy2(self.decisions, self.archive / 'decisions.json')

    def test_exact_canopy_chain(self):
        self.assertEqual(set(select(self.decisions, {'tree': self.base})), {'tree'})

    def test_changed_archive(self):
        (self.archive / 'worker.blend').write_bytes(b'other candidate')
        with self.assertRaisesRegex(ValueError, 'Archived canopy evidence'):
            select(self.decisions, {'tree': self.base})

    def test_changed_base(self):
        self.base.write_bytes(b'new geometry')
        with self.assertRaisesRegex(ValueError, 'current geometry'):
            select(self.decisions, {'tree': self.base})

    def test_proof_must_be_review_bound(self):
        proof = json.loads(self.proof.read_text())
        proof['receiver_names'] = ['Changed scope']
        self.proof.write_text(json.dumps(proof))
        with self.assertRaisesRegex(ValueError, 'reviewed preservation proof'):
            select(self.decisions, {'tree': self.base})

    def freeze_grouped(self, files):
        document = json.loads(self.decisions.read_text())
        row = document['decisions'][0]
        row.update(candidate=str(self.candidate), model_sha256=sha(self.model),
                   evidence_sha256={str(p): sha(p) for p in files}, archived_evidence={})
        for index, path in enumerate(files):
            archived = self.archive / f'grouped-{index}-{path.name}'
            shutil.copy2(path, archived)
            row['archived_evidence'][str(path)] = str(archived)
        self.decisions.write_text(json.dumps(document))
        shutil.copy2(self.decisions, self.archive / 'decisions.json')

    def grouped_review(self):
        root = self.candidate / 'root-review.json'
        root.write_text(json.dumps(dict(status='PASS scoped texture appearance',
            all_eight_actual_views_inspected=True, candidate_model_sha256=sha(self.model),
            actual_sheet_sha256=sha(self.sheet))))
        self.review.write_text(json.dumps(dict(all_eight_actual_views_inspected=True,
            candidate_model_sha256=sha(self.model), actual_sheet_sha256=sha(self.sheet),
            reopened_preservation_sha256=sha(self.proof))))
        self.freeze_grouped([self.model, self.sheet, self.review, self.proof, root])
        return root

    def test_grouped_actual_review_and_changed_guard(self):
        self.grouped_review()
        self.assertEqual(set(select(self.decisions, {'tree': self.base})), {'tree'})
        proof = json.loads(self.proof.read_text())
        proof['physical_alpha_unchanged'] = False
        self.proof.write_text(json.dumps(proof))
        with self.assertRaisesRegex(ValueError, 'Stale approved canopy evidence'):
            select(self.decisions, {'tree': self.base})

    def test_grouped_review_requires_all_views(self):
        root = self.grouped_review()
        data = json.loads(root.read_text())
        data['all_eight_actual_views_inspected'] = False
        root.write_text(json.dumps(data))
        self.freeze_grouped([self.model, self.sheet, self.review, self.proof, root])
        with self.assertRaises(ValueError):
            select(self.decisions, {'tree': self.base})

    def test_restored_observed_boundary_requires_parent_chain(self):
        parent_model = self.model
        self.candidate = self.root / 'restored'
        self.candidate.mkdir()
        self.model = self.candidate / 'worker.blend'
        self.model.write_bytes(b'exact observed RGB restored')
        (self.candidate / 'actual').mkdir()
        self.sheet = self.candidate / 'actual/textured.png'
        self.sheet.write_bytes(b'restored actual eight views')
        proof = self.candidate / 'native-boundary-preservation.json'
        data = dict(status='PASS scoped native boundary restoration',
            candidate_model_sha256=sha(self.model), source_model_sha256=sha(self.base),
            parent_candidate_sha256=sha(parent_model), geometry_uv_alpha_ownership_unchanged=True,
            other_materials_unchanged=True, native_boundary_rgba_exact=True, evidence_sha256={})
        proof.write_text(json.dumps(data))
        root = self.candidate / 'root-review.json'
        root.write_text(json.dumps(dict(status='PASS scoped texture appearance',
            all_eight_actual_views_inspected=True, candidate_model_sha256=sha(self.model),
            actual_sheet_sha256=sha(self.sheet))))
        self.freeze_grouped([self.model, self.sheet, proof, root])
        self.assertEqual(set(select(self.decisions, {'tree': self.base})), {'tree'})
        data['parent_candidate_sha256'] = 'wrong parent'
        proof.write_text(json.dumps(data))
        self.freeze_grouped([self.model, self.sheet, proof, root])
        with self.assertRaisesRegex(ValueError, 'parent bake changed'):
            select(self.decisions, {'tree': self.base})


if __name__ == '__main__':
    unittest.main()
