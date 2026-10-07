"""Negative checks for publication evidence and exact retirement coverage."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('publication', Path(__file__).with_name('restart7_apply_static_publication.py'))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

class PublicationGuards(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        def write(name, value):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value))
            return path
        self.write = write
        self.document = write('stage/croisement02.rhlos-map.json', {'placements': [{'assets': ['new']} ]})
        self.descriptor = write('library/old/asset.json', {'id': 'old', 'gameplay': {'version': 1}})
        self.source = write('stage/model.json', {'mesh': 'unchanged'})
        handoff = write('handoff.json', {'retire_ids': ['old']})
        semantic = write('semantic.json', {'pass': True})
        self.inputs = {str(self.document): MODULE.publisher.sha(self.document), str(self.source): MODULE.publisher.sha(self.source)}
        self.result = write('result.json', {'status': 'PASS', 'map_sha256': MODULE.publisher.sha(self.document), 'inputs': self.inputs})
        self.binding = write('binding.json', {'status': 'PASS', 'map_sha256': MODULE.publisher.sha(self.document), 'inputs': self.inputs, 'semantic_sha256': MODULE.publisher.sha(semantic)})
        self.manifest = dict(status='PREPARED_NOT_APPLIED', stage=str(self.document.parent), index_generation={'target': str(self.root/'library/index.json')},
            retirement_authority=dict(handoff=str(handoff), handoff_sha256=MODULE.publisher.sha(handoff), semantic_proof=str(semantic), semantic_sha256=MODULE.publisher.sha(semantic), descriptors=[dict(id='old', path=str(self.descriptor), sha256=MODULE.publisher.sha(self.descriptor))]),
            final_editor_proof=dict(status='PASS',result=str(self.result),semantic_binding=str(self.binding),files={str(self.result):MODULE.publisher.sha(self.result),str(self.binding):MODULE.publisher.sha(self.binding)}),
            files=[dict(source=str(self.document),target=str(self.root/'library/map.json'),source_sha256=MODULE.publisher.sha(self.document)),dict(source=str(self.source),target=str(self.root/'library/model.json'),source_sha256=MODULE.publisher.sha(self.source)),dict(source=None,target=str(self.descriptor))])
        self.path = self.root/'promotion.json'
    def check(self):
        self.path.write_text(json.dumps(self.manifest))
        return MODULE.validate(self.path)
    def test_complete_exact_evidence_passes(self):
        self.check()
    def test_changed_model_after_review_rejected(self):
        self.source.write_text('{}')
        with self.assertRaises(ValueError): self.check()
    def test_missing_retirement_rejected(self):
        self.manifest['files'].pop()
        with self.assertRaises(ValueError): self.check()
    def test_extra_unlisted_deletion_rejected(self):
        self.manifest['files'].append(dict(source=None,target=str(self.root/'unrelated/asset.json')))
        with self.assertRaises(ValueError): self.check()
    def test_empty_evidence_rejected(self):
        self.manifest['final_editor_proof']['files']={}
        with self.assertRaises(ValueError): self.check()
    def test_false_result_cannot_be_overridden_by_manifest_pass(self):
        data=json.loads(self.result.read_text());data['status']='FAIL';self.result.write_text(json.dumps(data))
        self.manifest['final_editor_proof']['files'][str(self.result)]=MODULE.publisher.sha(self.result)
        with self.assertRaises(ValueError): self.check()
    def test_proof_missing_transaction_input_rejected(self):
        data=json.loads(self.result.read_text());data['inputs'].pop(str(self.source));self.result.write_text(json.dumps(data))
        self.manifest['final_editor_proof']['files'][str(self.result)]=MODULE.publisher.sha(self.result)
        with self.assertRaises(ValueError): self.check()

if __name__ == '__main__': unittest.main()
