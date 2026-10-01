import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from publish_segments import prepare_runtime, sha
from publish_library import stage_library


class RuntimePublicationTests(unittest.TestCase):
    def setUp(self):
        temp=tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root=Path(temp.name)
        self.library=self.root/'library'
        self.stage=self.library/'3d-assets'
        self.folder=self.stage/'spline-test'
        self.folder.mkdir(parents=True)
        (self.library/'scenes').mkdir()
        (self.library/'game-data').mkdir()
        (self.library/'game-data/index.json').write_text(json.dumps({'version': 1, 'files': []}))
        (self.folder/'model.glb').write_bytes(b'authoring model')
        (self.folder/'asset.json').write_text(json.dumps({
            'id':'spline-test','name':'Test strip','source_map':'Derby','model':'model.glb'}))

    def derivatives(self, *_args, **_kwargs):
        raw=json.dumps({'asset':{'version':'2.0'}}).encode()
        raw+=b' '*(-len(raw)%4)
        glb=struct.pack('<5I',0x46546c67,2,20+len(raw),len(raw),0x4e4f534a)+raw
        for name,source in [('lossy.glb','model.glb'),('preview.glb','lossy.glb')]:
            (self.folder/name).write_bytes(glb)
            (self.folder/(name+'.receipt.json')).write_text(json.dumps({
                'source':sha(self.folder/source),'output':sha(self.folder/name),
                'source_model':'spline-test/'+source}))

    def prepare(self):
        return prepare_runtime(['spline-test'],stage=self.stage)

    def test_derivatives_and_receipts_are_carried_into_web_publication(self):
        with patch('publish_segments.subprocess.run',side_effect=self.derivatives) as run:
            files=self.prepare()['spline-test']
        self.assertIn('spline-test/lossy.glb.receipt.json',files)
        self.assertIn('spline-test/preview.glb.receipt.json',files)
        self.assertIn('refresh',run.call_args.args[0])
        self.assertTrue(run.call_args.kwargs['check'])
        report=stage_library(self.library,self.root/'upload')
        self.assertIn('3d-assets/spline-test/lossy.glb',report['payloads'])
        self.assertNotIn('3d-assets/spline-test/model.glb',report['payloads'])

    def test_refused_derivative_blocks_installation(self):
        with patch('publish_segments.subprocess.run'), self.assertRaisesRegex(ValueError,'Missing runtime derivative'):
            self.prepare()

    def test_failed_blender_blocks_installation(self):
        with patch('publish_segments.subprocess.run',side_effect=subprocess.CalledProcessError(1,'blender')):
            with self.assertRaises(subprocess.CalledProcessError):self.prepare()

    def test_stale_derivative_is_rejected(self):
        self.derivatives()
        (self.folder/'model.glb').write_bytes(b'changed model')
        with patch('publish_segments.subprocess.run'), self.assertRaisesRegex(ValueError,'receipt'):
            self.prepare()


if __name__=='__main__':unittest.main()
