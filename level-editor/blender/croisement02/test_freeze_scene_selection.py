"""Derivative camera metadata must remain bound to the approved model."""
import json
from pathlib import Path
import tempfile
import unittest

from evidence_io import sha
from freeze_scene_selection import source_frames


class SourceMetadataTests(unittest.TestCase):
    def test_explicit_companion_and_changed_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            worker = Path(directory) / 'worker'
            (worker / 'inspection').mkdir(parents=True)
            model = worker / 'model.blend'
            model.write_bytes(b'exact approved derivative')
            frames = Path(directory) / 'original-views.json'
            frames.write_bytes(b'original camera and ownership metadata')
            authority = worker / 'inspection/approved-geometry-authority.json'
            authority.write_text(json.dumps(dict(model_sha256=sha(model),
                source_authority=dict(manifest=str(frames), manifest_sha256=sha(frames),
                    purpose='Source metadata only; not a derivative geometry review'))))
            self.assertEqual(source_frames(worker), (frames, authority))
            frames.write_bytes(b'changed ownership')
            with self.assertRaisesRegex(ValueError, 'source metadata changed'):
                source_frames(worker)

    def test_no_inferred_fallback_to_old_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            worker = Path(directory)
            (worker / 'inspection').mkdir()
            (worker / 'inspection/approved-geometry-authority.json').write_text(
                json.dumps(dict(original_workspace='old worker')))
            with self.assertRaises(KeyError):
                source_frames(worker)


if __name__ == '__main__':
    unittest.main()
