#!/usr/bin/env python3
"""Provisioned lifecycle acceptance; never installs tools or changes player data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
REPLAY_CHECKS = {"native_playback_finished", "post_bootstrap_hash_verified"}
LIVE_CHECKS = {"normal_frame_and_manual_steps", "paused_manual_steps",
               "canonical_compact_export"}
SAVE_CHECKS = {"native_save_load_restored_state", "recording_continues_after_state_load",
               "save_load_post_restore_replay_hashes"}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_identity():
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT)

    # Separate index/worktree diffs detect changes cancelling in `git diff HEAD`.
    staged = git("diff", "--cached", "--binary")
    unstaged = git("diff", "--binary")
    return {
        "harness_source_commit": git("rev-parse", "HEAD").decode().strip(),
        "harness_source_tree": git("rev-parse", "HEAD^{tree}").decode().strip(),
        "tracked_source_dirty": bool(staged or unstaged),
        "index_diff_sha256": hashlib.sha256(staged).hexdigest(),
        "worktree_diff_sha256": hashlib.sha256(unstaged).hexdigest(),
        "untracked_paths": git("ls-files", "--others", "--exclude-standard", "-z").decode().split("\0")[:-1],
    }


def verify_source(initial, current, *, browser_build=False):
    if browser_build and (initial["tracked_source_dirty"] or initial["untracked_paths"]):
        raise RuntimeError("browser builds require a clean tracked and untracked source checkout")
    if initial != current:
        raise RuntimeError("source checkout changed during lifecycle acceptance; rerun on a frozen snapshot")


def executable(value):
    result = shutil.which(value)
    if not result:
        raise RuntimeError(f"required executable unavailable: {value}")
    return str(Path(result).resolve())


def run(argv, *, env=None, timeout=None, log=None):
    """Own the entire runtime process group, including timeout/failure cleanup."""
    print("+ " + " ".join(map(str, argv)), flush=True)
    output = log.open("w") if log else None
    child = subprocess.Popen(list(map(str, argv)), cwd=ROOT, env=env,
                             start_new_session=True, stdout=output,
                             stderr=subprocess.STDOUT if output else None)
    try:
        code = child.wait(timeout=timeout)
        if code:
            raise subprocess.CalledProcessError(code, argv)
    finally:
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        if output:
            output.close()
            print(log.read_text(), flush=True)


def lock_bindgen_version():
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    versions = {p["version"] for p in packages if p["name"] == "wasm-bindgen"}
    if len(versions) != 1:
        raise RuntimeError(f"expected one wasm-bindgen version, found {versions}")
    return versions.pop()


def browser(evidence, summary):
    chrome = executable(os.environ["CHROME"])
    driver = executable(os.environ["CHROMEDRIVER"])
    runner = executable(os.environ["WASM_BINDGEN_TEST_RUNNER"])
    version = subprocess.check_output([runner, "--version"], text=True).strip()
    if version.split()[-1] != lock_bindgen_version():
        raise RuntimeError(f"runner {version} does not match Cargo.lock wasm-bindgen")
    chrome_version = subprocess.check_output([chrome, "--version"], text=True).strip()
    driver_version = subprocess.check_output([driver, "--version"], text=True).strip()
    if re.search(r"\d+", chrome_version)[0] != re.search(r"\d+", driver_version)[0]:
        raise RuntimeError("Chrome and ChromeDriver major versions differ")
    summary.update(runner=version, chrome=chrome_version, chromedriver=driver_version)
    cargo = ["cargo", "--locked"]
    selection = ["--profile", "wasm-dev", "--target", "wasm32-unknown-unknown",
                 "-p", "robin_rs", "--no-default-features", "--features", "audio"]
    run([cargo[0], "check", cargo[1], *selection, "--bin", "robin", "--tests"])
    multiplayer_selection = ["audio,multiplayer" if item == "audio" else item for item in selection]
    run([cargo[0], "check", cargo[1], *multiplayer_selection, "--bin", "robin", "--tests"])
    # Cargo's JSON artifact event identifies the exact linked test executable;
    # never guess using glob order or a stale target-directory timestamp.
    argv = [cargo[0], "test", cargo[1], *selection, "--lib", "--no-run",
            "--message-format=json-render-diagnostics"]
    artifacts = []
    with subprocess.Popen(argv, cwd=ROOT, stdout=subprocess.PIPE, text=True) as child:
        for line in child.stdout:
            print(line, end="", flush=True)  # Preserve every Cargo output line.
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (event.get("reason") == "compiler-artifact"
                    and event.get("target", {}).get("name") == "robin_rs"
                    and event.get("profile", {}).get("test")
                    and event.get("executable")):
                artifacts.append(Path(event["executable"]))
        if child.wait():
            raise RuntimeError("WASM test-module link failed")
    if len(artifacts) != 1:
        raise RuntimeError(f"expected one linked client test module, got {artifacts}")
    module = evidence / "browser-tests.wasm"
    shutil.copyfile(artifacts[0], module)
    summary["wasm_sha256"] = digest(module)
    with tempfile.TemporaryDirectory(prefix="robin-audio-profile-") as profile:
        options = evidence / "webdriver.json"
        options.write_text(json.dumps({"goog:chromeOptions": {"binary": chrome, "args": [
            f"--user-data-dir={profile}", "--disable-background-networking",
            "--disable-component-update", "--disable-sync", "--disable-default-apps",
            "--no-first-run", "--no-proxy-server",
            "--host-resolver-rules=MAP * 127.0.0.1, EXCLUDE localhost"]}}))
        env = dict(os.environ, CHROMEDRIVER=driver,
                   WASM_BINDGEN_TEST_WEBDRIVER_JSON=str(options),
                   WASM_BINDGEN_TEST_TIMEOUT="120")
        env.pop("NO_HEADLESS", None)
        # Running all module tests includes audio ownership and future additions.
        log = evidence / "browser-tests.log"
        run([runner, module], env=env, timeout=240, log=log)
        result = re.search(r"test result: ok\. (\d+) passed; (\d+) failed", log.read_text())
        if not result or int(result[1]) == 0 or int(result[2]) != 0:
            raise RuntimeError("browser runner did not report a nonempty passing test suite")
        if not re.search(r"test web_audio_backend::[^\n]+\.\.\. ok", log.read_text()):
            raise RuntimeError("browser runner did not execute audio ownership tests")
        summary["browser_tests_passed"] = int(result[1])
    summary["checks"] = {"audio_target_check": True, "audio_multiplayer_target_check": True,
                         "test_module_link": True,
                         "real_browser_tests": True}


def native(evidence, summary):
    binary = Path(os.environ["ROBIN_LIFECYCLE_BINARY"]).resolve(strict=True)
    data = Path(os.environ["ROBINHOOD_DATA_DIR"]).resolve(strict=True)
    if not (data / "Data").is_dir():
        raise RuntimeError("ROBINHOOD_DATA_DIR must contain Data/")
    for tool in ("unshare", "ip", "Xvfb", "xdotool"):
        executable(tool)
    expected = os.environ["ROBIN_LIFECYCLE_BINARY_SHA256"]
    if digest(binary) != expected:
        raise RuntimeError("prebuilt binary does not match supplied SHA256")
    snapshot = os.environ["ROBIN_LIFECYCLE_SNAPSHOT"]
    summary.update(binary=str(binary), binary_sha256=expected,
                   binary_source_snapshot=snapshot, data=str(data), checks={})
    driver = ROOT / "scripts/validation/frame_steps_live.py"
    for name, extra in (("ordinary", []), ("save-load", ["--save-load"])):
        live = evidence / name
        for suffix, destination, flags in (("headless", live, extra),
                ("graphical", evidence / (name + "-graphical"),
                 ["--replay-file", live / "export.rhrec", "--graphical-replay"])):
            if digest(binary) != expected:
                raise RuntimeError("binary changed during acceptance")
            run(["unshare", "--user", "--map-root-user", "--net", sys.executable,
                 driver, "--binary", binary, "--data", data, "--snapshot", snapshot,
                 "--evidence", destination, *flags], timeout=330,
                env=dict(os.environ, PYTHONOPTIMIZE="0"))
            result = json.loads((destination / "summary.json").read_text())
            required = REPLAY_CHECKS | (LIVE_CHECKS if suffix == "headless" else set())
            if name == "save-load" and suffix == "headless":
                required |= SAVE_CHECKS
            if (result.get("completed") is not True or result.get("binary_sha256") != expected
                    or result.get("snapshot") != snapshot
                    or any(result.get("checks", {}).get(key) is not True for key in required)):
                raise RuntimeError(f"invalid acceptance summary: {destination}")
            if name == "save-load" and suffix == "graphical":
                from save_load_live import verify_replay
                saved = json.loads((live / "summary.json").read_text())["save_load"]
                verify_replay((destination / "playback.log").read_text(),
                              saved["load_record_frame"], saved["final_record_frame"])
                result["checks"]["save_load_post_restore_replay_hashes"] = True
            summary["checks"][name + "-" + suffix] = result
        summary[name + "_replay_sha256"] = digest(live / "export.rhrec")
    if digest(binary) != expected:
        raise RuntimeError("binary changed during acceptance")


def main():
    def interrupted(signum, _frame):
        raise InterruptedError(f"gate interrupted by signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["browser-audio", "native-lifecycle"])
    args = parser.parse_args()
    evidence = Path(os.environ.get("ROBIN_LIFECYCLE_EVIDENCE") or
                    tempfile.mkdtemp(prefix="robin-lifecycle-gate-" )).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    if any(evidence.iterdir()):
        raise RuntimeError("evidence directory must be empty")
    initial_source = source_identity()
    summary = {"suite": args.suite, "completed": False, **initial_source,
               "harness_sha256": digest(Path(__file__))}
    try:
        verify_source(initial_source, initial_source, browser_build=args.suite == "browser-audio")
        (browser if args.suite == "browser-audio" else native)(evidence, summary)
        final_source = source_identity()
        summary["final_source"] = final_source
        verify_source(initial_source, final_source)
        summary["completed"] = True
    except (Exception, KeyboardInterrupt) as error:
        summary["error"] = str(error)
    finally:
        (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(f"Lifecycle evidence: {evidence}", flush=True)
    if not summary["completed"]:
        print(summary["error"], file=sys.stderr)
    return 0 if summary["completed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
