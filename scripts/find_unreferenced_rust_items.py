#!/usr/bin/env python3
"""Find Rust functions/methods with no rust-analyzer references.

This is intentionally an LSP client instead of a text-search script. It asks
rust-analyzer for document symbols, then asks for references at each item
definition and filters out the definition location.

The output is a review queue, not a deletion list. Public API, trait methods,
macro-discovered entry points, tests, and callback-style functions can all look
unreferenced from the workspace's point of view.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time
from collections import deque
from dataclasses import dataclass
from pathlib import Path
from typing import Any


SYMBOL_KIND_NAMES = {
    6: "method",
    9: "constructor",
    12: "function",
}

DEFAULT_EXCLUDES = {
    ".git",
    ".claude",
    "target",
    "third_party",
    "wasm-www",
}


@dataclass(frozen=True)
class Item:
    name: str
    kind: int
    path: Path
    line: int
    character: int


@dataclass(frozen=True)
class Options:
    request_timeout: float
    verbose: bool


class ContentModifiedError(RuntimeError):
    pass


class LspClient:
    def __init__(self, command: list[str], options: Options) -> None:
        self._options = options
        self.proc = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        assert self.proc.stdin is not None
        assert self.proc.stdout is not None
        assert self.proc.stderr is not None
        self._stdin = self.proc.stdin
        self._stdout = self.proc.stdout
        self._stderr = self.proc.stderr
        self._next_id = 1
        self._messages: queue.Queue[dict[str, Any]] = queue.Queue()
        self._stderr_tail: deque[str] = deque(maxlen=40)
        self._send_lock = threading.Lock()
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._stderr_reader = threading.Thread(target=self._drain_stderr, daemon=True)
        self._reader.start()
        self._stderr_reader.start()

    def close(self) -> None:
        if self.proc.poll() is not None:
            return
        try:
            try:
                self.request("shutdown", None, timeout=10)
                self.notify("exit", None)
            except (BrokenPipeError, RuntimeError, TimeoutError):
                pass
        finally:
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def _read_loop(self) -> None:
        while True:
            headers: dict[str, str] = {}
            while True:
                line = self._stdout.readline()
                if not line:
                    return
                if line in (b"\r\n", b"\n"):
                    break
                key, _, value = line.decode("ascii").partition(":")
                headers[key.lower()] = value.strip()

            length = int(headers.get("content-length", "0"))
            body = self._stdout.read(length)
            if not body:
                return
            self._messages.put(json.loads(body))

    def _drain_stderr(self) -> None:
        for line in self._stderr:
            self._stderr_tail.append(line.decode("utf-8", errors="replace").rstrip())

    def _send(self, message: dict[str, Any]) -> None:
        body = json.dumps(message, separators=(",", ":")).encode("utf-8")
        header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
        with self._send_lock:
            self._stdin.write(header)
            self._stdin.write(body)
            self._stdin.flush()

    def notify(self, method: str, params: Any) -> None:
        message: dict[str, Any] = {
            "jsonrpc": "2.0",
            "method": method,
        }
        if params is not None:
            message["params"] = params
        self._send(message)

    def request(self, method: str, params: Any, timeout: float = 120) -> Any:
        if self._options.verbose:
            print(f"lsp request: {method}", file=sys.stderr)
        request_id = self._next_id
        self._next_id += 1
        message: dict[str, Any] = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
        }
        if params is not None:
            message["params"] = params
        self._send(message)

        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(self._failure_message(f"timed out waiting for {method}"))
            if self.proc.poll() is not None:
                raise RuntimeError(
                    self._failure_message(
                        f"rust-analyzer exited while waiting for {method}"
                    )
                )
            try:
                incoming = self._messages.get(timeout=min(remaining, 0.25))
            except queue.Empty:
                continue

            if incoming.get("id") == request_id:
                if "error" in incoming:
                    error = incoming["error"]
                    if error.get("code") == -32801:
                        raise ContentModifiedError(f"{method} failed: {error}")
                    raise RuntimeError(f"{method} failed: {error}")
                return incoming.get("result")

            self._handle_server_message(incoming)

    def drain_for(self, seconds: float) -> None:
        deadline = time.monotonic() + seconds
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            try:
                incoming = self._messages.get(timeout=remaining)
            except queue.Empty:
                return
            self._handle_server_message(incoming)

    def _handle_server_message(self, message: dict[str, Any]) -> None:
        if self._options.verbose and "method" in message and "id" not in message:
            print(f"lsp notification: {message['method']}", file=sys.stderr)
        if "id" in message and "method" in message:
            method = message["method"]
            if method == "workspace/configuration":
                result: Any = [{} for _ in message.get("params", {}).get("items", [])]
            else:
                result = None
            self._send({"jsonrpc": "2.0", "id": message["id"], "result": result})

    def _failure_message(self, message: str) -> str:
        stderr = "\n".join(self._stderr_tail)
        if stderr:
            return f"{message}\nrust-analyzer stderr tail:\n{stderr}"
        return message


def uri(path: Path) -> str:
    return path.resolve().as_uri()


def iter_rust_files(paths: list[Path], include_tests: bool) -> list[Path]:
    files: list[Path] = []
    for path in paths:
        if path.is_file() and path.suffix == ".rs":
            candidates = [path]
        else:
            candidates = path.rglob("*.rs")

        for candidate in candidates:
            parts = set(candidate.parts)
            if parts & DEFAULT_EXCLUDES:
                continue
            if not include_tests and (
                candidate.name == "tests.rs" or "tests" in candidate.parts
            ):
                continue
            files.append(candidate.resolve())
    return sorted(set(files))


def flatten_symbols(path: Path, symbols: list[dict[str, Any]]) -> list[Item]:
    items: list[Item] = []
    for symbol in symbols:
        kind = symbol.get("kind")
        if kind in SYMBOL_KIND_NAMES:
            if "selectionRange" in symbol:
                start = symbol["selectionRange"]["start"]
            elif "location" in symbol:
                start = symbol["location"]["range"]["start"]
            else:
                children = symbol.get("children") or []
                items.extend(flatten_symbols(path, children))
                continue
            items.append(
                Item(
                    name=symbol["name"],
                    kind=kind,
                    path=path,
                    line=start["line"],
                    character=start["character"],
                )
            )
        children = symbol.get("children") or []
        items.extend(flatten_symbols(path, children))
    return items


def source_line(path: Path, zero_based_line: int) -> str:
    try:
        return path.read_text(encoding="utf-8").splitlines()[zero_based_line].strip()
    except (OSError, UnicodeDecodeError, IndexError):
        return ""


def has_test_attr(path: Path, zero_based_line: int) -> bool:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return False

    idx = zero_based_line - 1
    while idx >= 0:
        line = lines[idx].strip()
        if not line:
            idx -= 1
            continue
        if line.startswith("#["):
            if "test" in line:
                return True
            idx -= 1
            continue
        return False
    return False


def request_with_content_retry(
    client: LspClient,
    method: str,
    params: Any,
    timeout: float,
) -> Any:
    for attempt in range(4):
        try:
            return client.request(method, params, timeout=timeout)
        except ContentModifiedError:
            if attempt == 3:
                raise
            time.sleep(0.25 * (attempt + 1))
    raise AssertionError("unreachable")


def is_definition_reference(ref: dict[str, Any], item: Item) -> bool:
    location = ref.get("location", ref)
    ref_uri = location.get("uri")
    start = location.get("range", {}).get("start", {})
    return (
        ref_uri == uri(item.path)
        and start.get("line") == item.line
        and start.get("character") == item.character
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Find Rust functions and methods with no rust-analyzer references."
    )
    parser.add_argument(
        "paths",
        nargs="*",
        type=Path,
        default=[Path("crates")],
        help="Rust files or directories to scan. Defaults to crates/.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="Workspace root passed to rust-analyzer. Defaults to cwd.",
    )
    parser.add_argument(
        "--rust-analyzer",
        default=os.environ.get("RUST_ANALYZER", "rust-analyzer"),
        help="rust-analyzer executable. Defaults to RUST_ANALYZER or rust-analyzer.",
    )
    parser.add_argument(
        "--include-tests",
        action="store_true",
        help="Include test files and tests/ directories.",
    )
    parser.add_argument(
        "--startup-delay",
        type=float,
        default=2.0,
        help="Seconds to let rust-analyzer process initial workspace notifications.",
    )
    parser.add_argument(
        "--request-timeout",
        type=float,
        default=120,
        help="Seconds to wait for each rust-analyzer request.",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print LSP request/notification progress to stderr.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON instead of human-readable lines.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    paths = [
        (root / path).resolve() if not path.is_absolute() else path
        for path in args.paths
    ]
    rust_files = iter_rust_files(paths, args.include_tests)
    if not rust_files:
        print("no Rust files found", file=sys.stderr)
        return 2

    client = LspClient(
        [args.rust_analyzer],
        Options(request_timeout=args.request_timeout, verbose=args.verbose),
    )
    try:
        client.request(
            "initialize",
            {
                "processId": os.getpid(),
                "rootUri": uri(root),
                "capabilities": {
                    "workspace": {"configuration": True},
                    "textDocument": {
                        "documentSymbol": {
                            "hierarchicalDocumentSymbolSupport": True,
                        },
                    },
                },
            },
            timeout=args.request_timeout,
        )
        client.notify("initialized", {})
        client.drain_for(args.startup_delay)

        all_items: list[Item] = []
        for path in rust_files:
            text = path.read_text(encoding="utf-8")
            client.notify(
                "textDocument/didOpen",
                {
                    "textDocument": {
                        "uri": uri(path),
                        "languageId": "rust",
                        "version": 1,
                        "text": text,
                    }
                },
            )
            symbols = request_with_content_retry(
                client,
                "textDocument/documentSymbol",
                {"textDocument": {"uri": uri(path)}},
                args.request_timeout,
            )
            all_items.extend(flatten_symbols(path, symbols or []))

        unreferenced: list[dict[str, Any]] = []
        for item in all_items:
            if not args.include_tests and has_test_attr(item.path, item.line):
                continue
            refs = request_with_content_retry(
                client,
                "textDocument/references",
                {
                    "textDocument": {"uri": uri(item.path)},
                    "position": {"line": item.line, "character": item.character},
                    "context": {"includeDeclaration": True},
                },
                args.request_timeout,
            )
            if any(not is_definition_reference(ref, item) for ref in refs or []):
                continue

            display_path = item.path.relative_to(root)
            unreferenced.append(
                {
                    "path": str(display_path),
                    "line": item.line + 1,
                    "column": item.character + 1,
                    "kind": SYMBOL_KIND_NAMES[item.kind],
                    "name": item.name,
                    "source": source_line(item.path, item.line),
                }
            )

        if args.json:
            print(json.dumps(unreferenced, indent=2))
        else:
            for item in unreferenced:
                print(
                    f"{item['path']}:{item['line']}:{item['column']}: "
                    f"{item['kind']} {item['name']} has no references"
                )
                if item["source"]:
                    print(f"    {item['source']}")
            print(
                f"\nscanned {len(all_items)} functions/methods in "
                f"{len(rust_files)} files; found {len(unreferenced)} with no references"
            )
    finally:
        client.close()

    return 1 if unreferenced else 0


if __name__ == "__main__":
    raise SystemExit(main())
