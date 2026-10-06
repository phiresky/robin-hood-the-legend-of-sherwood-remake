"""Exercise the catalog switch guard on temporary files only."""
import importlib.util,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('publication',Path(__file__).with_name('restart6_publish_remaining_states.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class SwitchGuard(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.oldlib=module.LIB;module.LIB=Path(self.tmp.name);self.index=module.LIB/'mission-states/index.json';self.index.parent.mkdir();self.index.write_bytes(b'old34');self.oldhash=module.sha(self.index)
 def tearDown(self):module.LIB=self.oldlib;self.tmp.cleanup()
 def test_exact_baseline_switches_atomically(self):
  module.replace_index(b'new41',self.oldhash);self.assertEqual(self.index.read_bytes(),b'new41');self.assertEqual(list(self.index.parent.iterdir()),[self.index])
 def test_different_baseline_refuses_without_writes(self):
  self.index.write_bytes(b'concurrent35')
  with self.assertRaisesRegex(AssertionError,'Concurrent catalog'):module.replace_index(b'new41',self.oldhash)
  self.assertEqual(self.index.read_bytes(),b'concurrent35');self.assertEqual(list(self.index.parent.iterdir()),[self.index])
 def test_resource_paths_cannot_escape_library(self):
  for path in ['/tmp/escape','mission-states/../../escape']:
   with self.assertRaises(ValueError):module.checked(module.LIB,path)
if __name__=='__main__':unittest.main()
