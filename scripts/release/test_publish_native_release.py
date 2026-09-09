import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import publish_native_release as release


class NativeReleaseTests(unittest.TestCase):
    def test_timestamp_nightly_tags_keep_commit_hash(self):
        for event in ("schedule", "workflow_dispatch"):
            self.assertEqual(release.release_tag(event, "main", "202609072359", "d01eb1295003abcdef"),
                             ("nightly-202609072359-d01eb1295003", True))
        self.assertEqual(release.release_tag("push", "v1.2.3", "", "commit"), ("v1.2.3", False))

    def test_retry_timestamp_comes_from_original_workflow_run(self):
        with patch.object(release, "gh", return_value='{"created_at":"2026-09-07T23:59:42Z"}') as gh:
            self.assertEqual(release.run_timestamp("owner/repo", "42"), "202609072359")
        gh.assert_called_once_with("api", "repos/owner/repo/actions/runs/42")

    def test_timestamp_normalizes_timezone_and_preserves_zeroes(self):
        with patch.object(release, "gh", return_value='{"created_at":"2026-09-08T02:05:00+02:00"}'):
            self.assertEqual(release.run_timestamp("owner/repo", "42"), "202609080005")

    def test_timestamp_requires_timezone(self):
        with patch.object(release, "gh", return_value='{"created_at":"2026-09-08T02:05:00"}'):
            with self.assertRaisesRegex(ValueError, "timezone"):
                release.run_timestamp("owner/repo", "42")

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
        draft = {"id": 42, "upload_url": "https://uploads.github.com/repos/owner/repo/releases/42/assets{?name,label}", "draft": True, "target_commitish": "commit", "assets": []}
        complete = {**draft, "assets": [{"name": "game", "id": 1}]}
        with patch.object(release, "inventory", return_value=assets), \
             patch.object(release, "find_release", return_value=draft), \
             patch.object(release, "remote_asset_sha256", return_value=assets["game"][1]) as digest, \
             patch.object(release, "gh", side_effect=["{}", json.dumps(complete), "{}"]) as gh:
            release.publish(Path("assets"), "owner/repo", "candidate", "commit", True)
        self.assertEqual(gh.call_args_list[0].args, ("api", "--method", "POST", "https://uploads.github.com/repos/owner/repo/releases/42/assets?name=game", "-H", "Content-Type: application/octet-stream", "--input", "game"))
        digest.assert_called_once_with("owner/repo", 1)
        self.assertEqual(gh.call_args_list[1].args, ("api", "repos/owner/repo/releases/42"))
        self.assertEqual(gh.call_args_list[2].args, ("api", "--method", "PATCH", "repos/owner/repo/releases/42", "-F", "draft=false"))

    def test_created_draft_need_not_appear_in_release_listing(self):
        assets = {"game": (Path("game"), hashlib.sha256(b"expected").hexdigest())}
        draft = {"id": 42, "upload_url": "https://uploads.github.com/repos/owner/repo/releases/42/assets{?name,label}",
                 "draft": True, "target_commitish": "commit", "assets": []}
        complete = {**draft, "assets": [{"name": "game", "id": 1}]}
        for prerelease in (True, False):
            with (
                self.subTest(prerelease=prerelease),
                patch.object(release, "inventory", return_value=assets),
                patch.object(release, "find_release", return_value=None) as find,
                patch.object(release, "remote_asset_sha256", return_value=assets["game"][1]),
                patch.object(release, "gh", side_effect=([] if prerelease else ["{}"]) +
                             [json.dumps(draft), "{}", json.dumps(complete), "{}"]) as gh,
            ):
                release.publish(Path("assets"), "owner/repo", "v1", "commit", prerelease)
            find.assert_called_once_with("owner/repo", "v1")
            calls = gh.call_args_list
            if not prerelease:
                self.assertEqual(calls.pop(0).args, ("api", "repos/owner/repo/git/ref/tags/v1"))
            self.assertEqual(calls[0].args[:4], ("api", "--method", "POST", "repos/owner/repo/releases"))
            self.assertIn(f"prerelease={str(prerelease).lower()}", calls[0].args)
            self.assertEqual(calls[-1].args, ("api", "--method", "PATCH", "repos/owner/repo/releases/42", "-F", "draft=false"))

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
