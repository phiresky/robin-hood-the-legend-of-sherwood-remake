import tempfile,unittest
from pathlib import Path
from unittest.mock import patch
from mound_contact_budget import source_name,digest,check,GIB,MIB,png_bound
class ContactBudgetTests(unittest.TestCase):
 def test_import_suffix_is_not_ambiguous_authority(self):
  self.assertEqual(source_name('Ground.004',['Ground','Bank']),'Ground')
  with self.assertRaises(ValueError):source_name('Ground.004',['Ground','Ground.001'])
 def test_finite_png_envelopes(self):
  self.assertLess(png_bound(480,480,4),MIB)
  self.assertLess(png_bound(1440,480,3),3*MIB)
  with self.assertRaises(ValueError):png_bound(480,480,16)
 def test_hash_is_streamed_exactly(self):
  with tempfile.TemporaryDirectory()as d:
   p=Path(d)/'x';p.write_bytes(b'abc');self.assertEqual(digest(p),'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
 def test_floor_reserves_next_write(self):
  with patch('mound_contact_budget.shutil.disk_usage')as disk,patch('mound_contact_budget.allocated',return_value=0):
   disk.return_value.free=10*GIB+MIB
   with self.assertRaises(RuntimeError):check('.', '.',2*MIB)
 def test_site_and_total_caps(self):
  with patch('mound_contact_budget.shutil.disk_usage')as disk,patch('mound_contact_budget.allocated',side_effect=[7*MIB]):
   disk.return_value.free=20*GIB
   with self.assertRaises(RuntimeError):check('.', '.',2*MIB)
  with patch('mound_contact_budget.shutil.disk_usage')as disk,patch('mound_contact_budget.allocated',side_effect=[MIB,159*MIB]):
   disk.return_value.free=20*GIB
   with self.assertRaises(RuntimeError):check('.', '.',2*MIB)
if __name__=='__main__':unittest.main()
