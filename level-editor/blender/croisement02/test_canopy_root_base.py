"""A reviewed root addition must stay bound to its approved tree and joint."""
import json
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from canopy_root_base import validate
from evidence_io import sha


class RootBaseTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        root=Path(self.temp.name)
        self.approved=root/'approved/croisement02-tree-15'
        self.worker=root/'root/croisement02-tree-15'
        self.bank=root/'bank'
        for path in [self.approved,self.worker,self.bank]:
            path.mkdir(parents=True);(path/'model.blend').write_bytes(str(path).encode())
        self.model_hash=sha(self.worker/'model.blend')
        self.joint=root/'joint.json';self.joint.write_text('{}')
        mask=root/'mask.png';mask.write_bytes(b'mask')
        inventory=root/'inventory.json';inventory.write_text(json.dumps(dict(masks=[dict(png=str(mask))])))
        records={
            'source-masks.json':dict(mask_inventory=str(inventory)),
            'inspection/root-preservation.json':dict(model_sha256=self.model_hash,
                previous_worker=str(self.approved),previous_model_sha256=sha(self.approved/'model.blend'),
                preserved=True,previous_meshes={'wood':'same'},current_meshes={'wood':'same'}),
            'inspection/root-component-review.json':dict(model_sha256=self.model_hash,root_component_ready=True,
                joint_evidence=str(self.joint),joint_evidence_sha256=sha(self.joint)),
            'inspection/root-completion.json':dict(model_sha256=self.model_hash,bank_worker=str(self.bank),bank_model_sha256=sha(self.bank/'model.blend')),
            'inspection/saved-model-audit.json':dict(model_sha256=self.model_hash,status='PASS'),
            'inspection/root-source-coverage/report.json':dict(model_sha256=self.model_hash,source_coverage=.963),
        }
        for path,data in records.items():
            target=self.worker/path;target.parent.mkdir(parents=True,exist_ok=True);target.write_text(json.dumps(data))

    def test_current_derivative_pins_external_evidence(self):
        record=validate(self.worker,self.approved)
        self.assertEqual(record['model_sha256'],self.model_hash)
        self.assertIn(str(self.joint),record['evidence_sha256'])

    def test_changed_approved_input(self):
        (self.approved/'model.blend').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'approved base'):validate(self.worker,self.approved)

    def test_changed_joint_bank(self):
        (self.bank/'model.blend').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'bank geometry changed'):validate(self.worker,self.approved)

    def test_insufficient_local_coverage(self):
        p=self.worker/'inspection/root-source-coverage/report.json'
        p.write_text(json.dumps(dict(model_sha256=self.model_hash,source_coverage=.70)))
        with self.assertRaisesRegex(ValueError,'local review'):validate(self.worker,self.approved)


if __name__=='__main__':unittest.main()
