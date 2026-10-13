import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from consolidate_archived_duplicates import plan, apply, edit

class ArchiveConsolidationTests(unittest.TestCase):
    def fixture(self, root):
        files=[]
        payload=b'archive exact bytes'
        for name in ['first/worker.blend','second/worker.blend']:
            p=root/'work/needs-review'/name;p.parent.mkdir(parents=True);p.write_bytes(payload);s=p.stat()
            files.append(dict(path=str(p.relative_to(root)),dev=s.st_dev,ino=s.st_ino,bytes=s.st_size,mtime_ns=s.st_mtime_ns,nlink=s.st_nlink,allocated_bytes=s.st_blocks*512))
        audit=root/'audit.json';audit.write_text(json.dumps({'groups':[{'sha256':hashlib.sha256(payload).hexdigest(),'files':files}]}))
        return audit,[root/r['path'] for r in files]

    def test_consumers_keep_paths_hashes_and_relative_context_then_edit_detaches(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);audit,paths=self.fixture(root)
            for i,p in enumerate(paths):(p.parent/'texture.png').write_bytes(str(i).encode())
            result=plan(root,[audit]);planfile=root/'plan.json';planfile.write_text(json.dumps(result))
            done=apply(planfile,root/'receipt.jsonl')
            self.assertEqual(done['deleted_paths'],0)
            self.assertGreater(done['reclaimed_bytes'],0)
            self.assertEqual(paths[0].stat().st_ino,paths[1].stat().st_ino)
            for i,p in enumerate(paths):
                self.assertEqual(p.read_bytes(),b'archive exact bytes')
                self.assertEqual((p.parent/'texture.png').read_bytes(),str(i).encode())
                self.assertEqual(p.stat().st_mode&0o222,0)
                if os.geteuid()!=0:
                    with self.assertRaises(PermissionError):p.write_bytes(b'accidental overwrite')
            edit(paths[1]);paths[1].write_bytes(b'new independent model')
            self.assertEqual(paths[0].read_bytes(),b'archive exact bytes')
            self.assertNotEqual(paths[0].stat().st_ino,paths[1].stat().st_ino)
            self.assertEqual(paths[0].stat().st_mode&0o222,0)

    def test_external_hardlinks_are_excluded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);audit,paths=self.fixture(root);os.link(paths[0],root/'active-model.blend')
            result=plan(root,[audit]);self.assertFalse(result['groups']);self.assertIn('outside',result['skipped'][0]['reason'])
            self.assertTrue(paths[0].stat().st_mode&0o200)

    def test_symlink_and_stale_identity_are_excluded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);audit,paths=self.fixture(root);paths[0].write_bytes(b'changed')
            result=plan(root,[audit]);self.assertFalse(result['groups'])
            paths[0].unlink();paths[0].symlink_to(paths[1]);self.assertFalse(plan(root,[audit])['groups'])

    def test_hash_guard_refuses_same_size_changed_bytes_even_with_old_mtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);audit,paths=self.fixture(root);result=plan(root,[audit]);p=root/'plan.json';p.write_text(json.dumps(result))
            s=paths[1].stat();paths[1].write_bytes(b'x'*s.st_size);os.utime(paths[1],ns=(s.st_atime_ns,s.st_mtime_ns))
            with self.assertRaisesRegex(ValueError,'hash'):apply(p,root/'receipt.jsonl')
            self.assertEqual(paths[0].read_bytes(),b'archive exact bytes')
            self.assertTrue(paths[0].stat().st_mode&0o200)

if __name__=='__main__':unittest.main()
