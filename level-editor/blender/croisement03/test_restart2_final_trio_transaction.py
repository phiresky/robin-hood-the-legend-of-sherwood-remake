"""Sandbox-only fault injection for durable root publication and evidence gates."""
import importlib.util,json,tempfile,unittest
from pathlib import Path
SCRIPT=Path(__file__).with_name('restart2_apply_final_trio_transaction.py')
spec=importlib.util.spec_from_file_location('c3_transaction',SCRIPT);tx=importlib.util.module_from_spec(spec);spec.loader.exec_module(tx)

class TransactionTests(unittest.TestCase):
 def test_replacement_deletion_creation_rollback(self):
  with tempfile.TemporaryDirectory()as name:
   d=Path(name);old=d/'old';old.write_bytes(b'old');obsolete=d/'obsolete';obsolete.write_bytes(b'derivative');new=d/'new';source=d/'source';source.write_bytes(b'candidate')
   rows=[{'source':str(source),'target':str(old),'old_sha256':tx.sha(old),'new_sha256':tx.sha(source)}, {'source':None,'target':str(obsolete),'old_sha256':tx.sha(obsolete),'new_sha256':None},{'source':str(source),'target':str(new),'old_sha256':None,'new_sha256':tx.sha(source)}]
   with self.assertRaisesRegex(RuntimeError,'Injected failure'):tx.protocol.commit(rows,d/'journal','plan',lambda:None,lambda:None,fail_after=3)
   self.assertEqual(old.read_bytes(),b'old');self.assertEqual(obsolete.read_bytes(),b'derivative');self.assertFalse(new.exists());self.assertEqual(tx.read(d/'journal/journal.json')['phase'],'ROLLED_BACK')
 def test_external_drift_refused_before_write(self):
  with tempfile.TemporaryDirectory()as name:
   d=Path(name);target=d/'target';target.write_bytes(b'changed');source=d/'source';source.write_bytes(b'new')
   rows=[{'source':str(source),'target':str(target),'old_sha256':'0'*64,'new_sha256':tx.sha(source)}]
   with self.assertRaisesRegex(RuntimeError,'Drift before backup'):tx.protocol.commit(rows,d/'journal','plan',lambda:None,lambda:None)
   self.assertEqual(target.read_bytes(),b'changed')
 def test_interaction_proof_requires_all_checks_and_exact_models(self):
  with tempfile.TemporaryDirectory()as name:
   d=Path(name);artifact=d/'artifact';artifact.write_bytes(b'actual evidence');proof=d/'proof.json';plan={'map_sha256':'a'*64,'approved_models':{'tree':'b'*64}};row={'status':'PASS','stage_map_sha256':plan['map_sha256'],'approved_models':plan['approved_models'],'editor_checks':{k:True for k in ('map_load','group_selection','part_selection','palette_insert','save_reload')},'evidence':{str(artifact):tx.sha(artifact)}}
   # Evidence pin verification deliberately restricts publication inputs to the workspace.
   row['editor_checks']['save_reload']=False;proof.write_text(json.dumps(row))
   with self.assertRaisesRegex(RuntimeError,'Missing editor interaction'):tx.editor_proof(proof,tx.sha(proof),plan)
   row['editor_checks']['save_reload']=True;row['approved_models']={'tree':'c'*64};proof.write_text(json.dumps(row))
   with self.assertRaisesRegex(RuntimeError,'model scope differs'):tx.editor_proof(proof,tx.sha(proof),plan)
if __name__=='__main__':unittest.main()
