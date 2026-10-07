import hashlib,json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import restart16_state_transaction as tx

class TransactionTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
 def test_absent_publish_preserves_exact_and_rejects_conflict(self):
  target=self.root/'resources/p.png';data=b'approved';digest=hashlib.sha256(data).hexdigest()
  self.assertTrue(tx.atomic_absent(target,data,digest));inode=target.stat().st_ino
  self.assertFalse(tx.atomic_absent(target,data,digest));self.assertEqual(inode,target.stat().st_ino)
  with self.assertRaises(ValueError):tx.atomic_absent(target,b'foreign',hashlib.sha256(b'foreign').hexdigest())
  self.assertEqual(target.read_bytes(),data)
 def test_partial_temp_write_never_exposes_destination(self):
  target=self.root/'p';data=b'approved'
  with patch.object(tx.os,'fsync',side_effect=OSError('interrupted')):
   with self.assertRaises(OSError):tx.atomic_absent(target,data,hashlib.sha256(data).hexdigest())
  self.assertFalse(target.exists());self.assertEqual(list(self.root.iterdir()),[])
 def test_racing_immutable_writer_is_not_overwritten(self):
  target=self.root/'p';real_link=tx.os.link
  def race(source,dest):
   Path(dest).write_bytes(b'foreign');return real_link(source,dest)
  with patch.object(tx.os,'link',side_effect=race):
   with self.assertRaises(ValueError):tx.atomic_absent(target,b'approved',hashlib.sha256(b'approved').hexdigest())
  self.assertEqual(target.read_bytes(),b'foreign')
 def test_index_drift_rejected_before_replace(self):
  index=self.root/'index';index.write_bytes(b'old');old=tx.sha(index);real_fsync=tx.os.fsync
  def drift(fd):
   real_fsync(fd);index.write_bytes(b'external')
  with patch.object(tx.os,'fsync',side_effect=drift):
   with self.assertRaises(ValueError):tx.switch_index(index,b'new',old)
  self.assertEqual(index.read_bytes(),b'external')
 def test_rollback_requires_owned_exact_current_index(self):
  index=self.root/'index';index.write_bytes(b'ours');new=tx.sha(index)
  receipt=dict(status='INSTALLED_PENDING_NORMAL_HTTP_PROOF',plan_sha256=tx.PLAN_SHA,index_sha256=new)
  index.write_bytes(b'external')
  with self.assertRaises(ValueError):tx.rollback_owned(index,b'old',new,receipt)
  self.assertEqual(index.read_bytes(),b'external');index.write_bytes(b'ours');tx.rollback_owned(index,b'old',new,receipt);self.assertEqual(index.read_bytes(),b'old')
 def test_path_traversal_and_symlink_rejected(self):
  for value in ['../escape','/absolute']:
   with self.assertRaises(ValueError):tx.checked(self.root,value)
  (self.root/'link').symlink_to(self.root,target_is_directory=True)
  with self.assertRaises(ValueError):tx.checked(self.root,'link/item')
 def test_gate_cannot_be_omitted(self):
  with self.assertRaises(ValueError):tx.verify_gate(None,None,{})

if __name__=='__main__':unittest.main()
