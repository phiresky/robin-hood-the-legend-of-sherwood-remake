"""Cleanup selectors must not reuse stale reviews or changed source ownership."""
import json
from pathlib import Path
import tempfile
import unittest
from canopy_candidates import selected_workspace, validate_worker
from evidence_io import sha


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.worker=self.root/'candidate';self.worker.mkdir()
        (self.worker/'model.blend').write_bytes(b'candidate')
        self.model_hash=sha(self.worker/'model.blend')
        self.previous=self.root/'previous';self.previous.mkdir();(self.previous/'model.blend').write_bytes(b'original')
        records={'validation.json':dict(status='PASS'),
            'inspection/saved-model-audit.json':dict(status='PASS',model_sha256=self.model_hash),
            'inspection/source-coverage/report.json':dict(model_sha256=self.model_hash,intersection_over_union=.99),
            'inspection/actual-materials/opacity-bounds.json':dict(model_sha256=self.model_hash,crowns=[dict(depth_width_ratio=1.2)]),
            'inspection/visual-review.json':dict(model_sha256=self.model_hash,ready_for_geometry_review=True)}
        for relative,record in records.items():
            path=self.worker/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(record))
        self.catalog=self.root/'catalog.json'
        self.catalog.write_text(json.dumps(dict(groups=[dict(id='croisement02-tree-00',parts=[dict(obstacle=44)])])))
        self.receipt=self.root/'canopy-cleanup-selections/tree-00.json';self.receipt.parent.mkdir()
        self.receipt.write_text(json.dumps(dict(asset_id='croisement02-tree-00',worker=str(self.worker),approval='pending',
            model_sha256=self.model_hash,previous_worker=str(self.previous),previous_model_sha256=sha(self.previous/'model.blend'),part_ids=['building-044'],evidence_sha256={str(self.worker/p):sha(self.worker/p) for p in records})))

    def test_valid_selection(self):
        self.assertEqual(selected_workspace(self.root,0,self.catalog),self.worker)

    def test_absent_selection_is_not_guessed(self):
        self.assertIsNone(selected_workspace(self.root,1,self.catalog))

    def test_changed_source_ownership(self):
        self.catalog.write_text(json.dumps(dict(groups=[dict(id='croisement02-tree-00',parts=[dict(obstacle=45)])])))
        with self.assertRaisesRegex(ValueError,'source ownership changed'):
            selected_workspace(self.root,0,self.catalog)

    def test_changed_review_is_rejected(self):
        (self.worker/'inspection/visual-review.json').write_text('{}')
        with self.assertRaisesRegex(ValueError,'evidence changed'):
            selected_workspace(self.root,0,self.catalog)

    def test_independent_prior_worker_changes(self):
        (self.previous/'model.blend').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'prior canopy worker changed'):
            selected_workspace(self.root,0,self.catalog)

    def test_depth_guard(self):
        path=self.worker/'inspection/actual-materials/opacity-bounds.json'
        path.write_text(json.dumps(dict(model_sha256=self.model_hash,crowns=[dict(depth_width_ratio=.8)])))
        with self.assertRaisesRegex(ValueError,'source or volume limits'):
            validate_worker(self.worker)


if __name__=='__main__':unittest.main()
