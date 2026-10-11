"""Read-only regression checks for recovered-proof publication guards."""
import json,unittest
from pathlib import Path
from unittest.mock import patch
import restart113_publish_climbing_tree42 as pub

class Guards(unittest.TestCase):
 @classmethod
 def setUpClass(cls):
  cls.path=pub.RUNS/'proposal-v2/plan.json';cls.plan=pub.read(cls.path)
 def authority(self):
  return dict(status='AUTHORIZE_SCOPED_CLIMBING_TREE42_INSTALL',plan_sha256=pub.sha(self.path),accepts_recovered_ui_limitations=True,requires_postinstall_current_runtime_ui=True,accepted_runtime_drift_paths=[r['path'] for r in self.plan['runtime_drift']])
 def test_read_only_preflight(self):pub.validate(self.plan,self.path,self.authority())
 def test_unreviewed_drift_rejected(self):
  a=self.authority();a['accepted_runtime_drift_paths']=[]
  with self.assertRaises(Exception):pub.validate(self.plan,self.path,a)
 def test_wrong_plan_authority_rejected(self):
  a=self.authority();a['plan_sha256']='0'*64
  with self.assertRaises(Exception):pub.validate(self.plan,self.path,a)
 def test_postinstall_obligation_required(self):
  a=self.authority();a['requires_postinstall_current_runtime_ui']=False
  with self.assertRaises(Exception):pub.validate(self.plan,self.path,a)
 def test_current_state_runtime_drift_rejected(self):
  old=pub.sha;target=pub.ROOT/'level-editor/app/src/state-delivery.ts'
  with patch.object(pub,'sha',side_effect=lambda p:'0'*64 if Path(p)==target else old(p)):
   with self.assertRaisesRegex(Exception,'Current runtime drift'):pub.validate(self.plan,self.path,self.authority())
 def test_catalog_drift_rejected(self):
  old=pub.io.sha;target=pub.LIB/'mission-states/index.json'
  with patch.object(pub.io,'sha',side_effect=lambda p:'0'*64 if Path(p)==target else old(p)):
   with self.assertRaisesRegex(Exception,'Target drift'):pub.validate(self.plan,self.path,self.authority())

if __name__=='__main__':unittest.main()
