import json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import restart58_gate_catalog_transaction as tx
class GateTransactionTests(unittest.TestCase):
 def setup_case(self,folder):
  root=Path(folder);lib=root/'live';stage=root/'stage';lib.mkdir();stage.mkdir();(lib/'mission-states').mkdir();(stage/'library/mission-states/york').mkdir(parents=True)
  before={'version':1,'custom':'retained','entries':[{'id':'unrelated-new-session','value':13}]};(lib/'mission-states/index.json').write_text(json.dumps(before))
  model=stage/'library/mission-states/york/test.glb';model.write_bytes(b'pinned model');entry={'id':'york-portcullis','contract':{'path':'mission-states/york/test.json','sha256':'a'*64}}
  rows=[{'source':str(model),'relative':'mission-states/york/test.glb','sha256':tx.sha(model)}];(stage/'plan.json').write_text(json.dumps({'entry':entry,'records':rows}));return lib,stage,before,entry,model
 def test_rebase_preserves_unrelated_latest_rows_and_switches_catalog_last(self):
  with tempfile.TemporaryDirectory() as d:
   lib,stage,before,entry,_=self.setup_case(d)
   with patch.object(tx,'LIB',lib),patch.object(tx,'STAGE',stage),patch.object(tx,'STAGE_SHA',tx.sha(stage/'plan.json')):
    result=tx.prepare(Path(d)/'run');self.assertEqual(json.loads((Path(d)/'run/index.json').read_text()),{**before,'entries':before['entries']+[entry]});self.assertEqual(result['records'][-1]['target'],str(lib/'mission-states/index.json'));self.assertEqual(json.loads((lib/'mission-states/index.json').read_text()),before)
 def test_conflicting_immutable_resource_refuses_before_plan(self):
  with tempfile.TemporaryDirectory() as d:
   lib,stage,_,_,_=self.setup_case(d);dest=lib/'mission-states/york/test.glb';dest.parent.mkdir();dest.write_bytes(b'external work')
   with patch.object(tx,'LIB',lib),patch.object(tx,'STAGE',stage),patch.object(tx,'STAGE_SHA',tx.sha(stage/'plan.json')):
    with self.assertRaisesRegex(Exception,'Conflicting immutable'):tx.prepare(Path(d)/'run')
   self.assertEqual(dest.read_bytes(),b'external work');self.assertFalse((Path(d)/'run').exists())
 def test_source_drift_refuses_before_plan(self):
  with tempfile.TemporaryDirectory() as d:
   lib,stage,_,_,model=self.setup_case(d);model.write_bytes(b'mutated')
   with patch.object(tx,'LIB',lib),patch.object(tx,'STAGE',stage),patch.object(tx,'STAGE_SHA',tx.sha(stage/'plan.json')):
    with self.assertRaisesRegex(Exception,'Stage payload drift'):tx.prepare(Path(d)/'run')
   self.assertFalse((Path(d)/'run').exists())
if __name__=='__main__':unittest.main()
