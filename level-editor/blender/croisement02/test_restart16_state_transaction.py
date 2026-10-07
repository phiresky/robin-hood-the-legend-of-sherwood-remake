import hashlib,io,json,tempfile,unittest
from contextlib import redirect_stdout
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
   with self.assertRaises(ValueError):tx.switch_index(index,b'new',old,hashlib.sha256(b'new').hexdigest())
  self.assertEqual(index.read_bytes(),b'external')
 def test_rollback_requires_owned_exact_current_index(self):
  index=self.root/'index';index.write_bytes(b'ours');new=tx.sha(index)
  old=hashlib.sha256(b'old').hexdigest();receipt=dict(status='INSTALLED_PENDING_NORMAL_HTTP_PROOF',plan_sha256=tx.PLAN_SHA,index_sha256=new,baseline_index_sha256=old)
  index.write_bytes(b'external')
  with self.assertRaises(ValueError):tx.rollback_owned(index,b'old',new,receipt,old)
  self.assertEqual(index.read_bytes(),b'external');index.write_bytes(b'ours');tx.rollback_owned(index,b'old',new,receipt,old);self.assertEqual(index.read_bytes(),b'old')
 def test_exact_staged_index_bytes_drift_cannot_switch(self):
  index=self.root/'index';index.write_bytes(b'old');before=tx.sha(index)
  expected_new=hashlib.sha256(b'approved staged index').hexdigest()
  with self.assertRaises(ValueError):tx.switch_index(index,b'stage changed after validation',before,expected_new)
  self.assertEqual(index.read_bytes(),b'old');self.assertEqual(list(self.root.iterdir()),[index])
 def test_cli_rollback_survives_missing_stage_and_resource_drift(self):
  plan_dir=self.root/'plan';plan_dir.mkdir();library=self.root/'library';(library/'mission-states').mkdir(parents=True)
  index=library/tx.INDEX;index.write_bytes(b'owned41');new=tx.sha(index);baseline=b'pinned34';old=hashlib.sha256(baseline).hexdigest();(plan_dir/'installed34-index.json').write_bytes(baseline)
  resource=library/'mission-states/changed-resource';resource.write_bytes(b'external drift must remain')
  plan={'installed_index_sha256':old,'baseline_index':{'sha256':old},'staged_index':{'sha256':new},'manifest':{'path':'missing-stage-manifest','sha256':'unavailable'}}
  (plan_dir/'plan.json').write_text(json.dumps(plan));plan_hash=tx.sha(plan_dir/'plan.json')
  installation=self.root/'installation.json';installation.write_text(json.dumps({'status':'INSTALLED_PENDING_NORMAL_HTTP_PROOF','plan_sha256':plan_hash,'index_sha256':new,'baseline_index_sha256':old}));receipt=self.root/'rollback.json'
  args=['transaction','--rollback','--installation',str(installation),'--installation-sha',tx.sha(installation),'--receipt',str(receipt)]
  with patch.object(tx,'PLAN',plan_dir),patch.object(tx,'LIB',library),patch.object(tx,'PLAN_SHA',plan_hash),patch('sys.argv',args),redirect_stdout(io.StringIO()):tx.main()
  self.assertEqual(index.read_bytes(),baseline);self.assertEqual(resource.read_bytes(),b'external drift must remain');self.assertEqual(json.loads(receipt.read_text())['status'],'ROLLED_BACK_INDEX_ONLY')
 def test_path_traversal_and_symlink_rejected(self):
  for value in ['../escape','/absolute']:
   with self.assertRaises(ValueError):tx.checked(self.root,value)
  (self.root/'link').symlink_to(self.root,target_is_directory=True)
  with self.assertRaises(ValueError):tx.checked(self.root,'link/item')
 def test_gate_cannot_be_omitted(self):
  with self.assertRaises(ValueError):tx.verify_gate(None,None,{})

if __name__=='__main__':unittest.main()
