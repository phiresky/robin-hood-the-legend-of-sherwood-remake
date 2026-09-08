import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import publish_native_release as release


class NativeReleaseTests(unittest.TestCase):
    def test_unique_retryable_nightly_tags(self):
        self.assertEqual(release.release_tag("schedule", "main", "42", "2026-09-07"),
                         ("nightly-2026-09-07-42", True))
        self.assertEqual(release.release_tag("push", "v1.2.3", "42", "2026-09-07"), ("v1.2.3", False))

    def test_retry_date_comes_from_original_workflow_run(self):
        with patch.object(release, "gh", return_value='{"created_at":"2026-09-07T23:59:00Z"}'):
            self.assertEqual(release.run_date("owner/repo", "42"), "2026-09-07")

    def test_inventory_requires_update_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("robin-windows-x86_64.zip", "robin-linux-x86_64.tar.gz", "game.nupkg"):
                (root / name).write_bytes(b"package")
            for runtime in ("win", "linux"):
                (root / f"releases.{runtime}.json").write_text(json.dumps({"Assets": [{
                    "FileName": "game.nupkg", "Size": 7,
                    "SHA256": hashlib.sha256(b"package").hexdigest(),
                }]}))
            self.assertEqual(len(release.inventory(root)), 5)
            (root / "game.nupkg").write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "size/hash differs"):
                release.inventory(root)
            (root / "game.nupkg").unlink()
            with self.assertRaisesRegex(ValueError, "missing asset"):
                release.inventory(root)

    def test_hash_mismatch_blocks_promotion(self):
        assets = {"game": (Path("game"), hashlib.sha256(b"expected").hexdigest())}
        remote = {"assets": [{"name": "game", "id": 1}]}
        with patch.object(release, "remote_asset_sha256", return_value=hashlib.sha256(b"wrong").hexdigest()):
            with self.assertRaisesRegex(ValueError, "differs"):
                release.verify_remote("owner/repo", remote, assets, allow_missing=True)

    def test_resume_uploads_missing_then_verifies_before_promotion(self):
        assets = {"game": (Path("game"), hashlib.sha256(b"expected").hexdigest())}
        draft = {"draft": True, "target_commitish": "commit", "assets": []}
        complete = {**draft, "assets": [{"name": "game", "id": 1}]}
        with patch.object(release, "inventory", return_value=assets), \
             patch.object(release, "find_release", side_effect=[draft, complete]), \
             patch.object(release, "remote_asset_sha256", return_value=assets["game"][1]) as digest, \
             patch.object(release, "gh", return_value="") as gh:
            release.publish(Path("assets"), "owner/repo", "candidate", "commit", True)
        self.assertEqual(gh.call_args_list[0].args[:2], ("release", "upload"))
        digest.assert_called_once_with("owner/repo", 1)
        self.assertEqual(gh.call_args_list[1].args[:2], ("release", "edit"))

    def test_published_release_is_never_modified(self):
        assets = {"game": (Path("game"), hashlib.sha256(b"expected").hexdigest())}
        complete = {"draft": False, "target_commitish": "commit", "assets": [{"name": "game", "id": 1}]}
        with patch.object(release, "inventory", return_value=assets), \
             patch.object(release, "find_release", return_value=complete), \
             patch.object(release, "remote_asset_sha256", return_value=assets["game"][1]), \
             patch.object(release, "gh") as gh:
            release.publish(Path("assets"), "owner/repo", "v1", "commit", False)
        gh.assert_not_called()


if __name__ == "__main__":
    unittest.main()
