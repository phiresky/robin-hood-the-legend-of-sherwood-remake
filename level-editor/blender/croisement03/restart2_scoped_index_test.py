import hashlib
from pathlib import Path
import tempfile
import unittest
from restart2_scoped_index import merge_scope, check_prior

class ScopeTests(unittest.TestCase):
    def test_legacy_cache_and_exact_replacement(self):
        old={'id':'unrelated','model':'same.glb','editor':{'legacy':'retain','name':'old'}}
        prior={'version':1,'assets':[old,{'id':'old049'}]}
        generated={'version':1,'assets':[dict(old,editor={'name':'old'}),{'id':'approved','model':'new.glb'}]}
        result=merge_scope(prior,generated,{'approved'},'old049')
        self.assertEqual({a['id']:a for a in result['assets']},{'unrelated':old,'approved':generated['assets'][1]})
        generated['assets'][0]['model']='changed.glb'
        with self.assertRaises(AssertionError):merge_scope(prior,generated,{'approved'},'old049')
    def test_stale_preflight_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'index.json';p.write_bytes(b'initial');expected=hashlib.sha256(p.read_bytes()).hexdigest()
            check_prior(p,expected);p.write_bytes(b'other publisher')
            with self.assertRaisesRegex(AssertionError,'Stale'):check_prior(p,expected)

if __name__=='__main__':unittest.main()
