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
import contextlib
import json
import os
import queue
import re
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
class Finding:
    path: str
    line: int
    column: int
    kind: str
    name: str
    source: str

    def as_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "line": self.line,
            "column": self.column,
            "kind": self.kind,
            "name": self.name,
            "source": self.source,
        }

    def format(self) -> str:
        lines = [
            f"{self.path}:{self.line}:{self.column}: "
            f"{self.kind} {self.name} has no references"
        ]
        if self.source:
            lines.append(f"    {self.source}")
        return "\n".join(lines)


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
        self._progress_tokens: set[str] = set()
        self._saw_progress = False
        self._last_progress = time.monotonic()
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

    def wait_for_work_done_progress(self, timeout: float, idle: float) -> None:
        deadline = time.monotonic() + timeout
        while True:
            now = time.monotonic()
            if (
                self._saw_progress
                and not self._progress_tokens
                and now - self._last_progress >= idle
            ):
                return
            remaining = deadline - now
            if remaining <= 0:
                if self._options.verbose:
                    print("workspace progress wait timed out", file=sys.stderr)
                return
            try:
                incoming = self._messages.get(timeout=min(remaining, 0.25))
            except queue.Empty:
                continue
            self._handle_server_message(incoming)

    def _handle_server_message(self, message: dict[str, Any]) -> None:
        if self._options.verbose and "method" in message and "id" not in message:
            print(f"lsp notification: {message['method']}", file=sys.stderr)
        if message.get("method") == "$/progress":
            params = message.get("params", {})
            token = str(params.get("token", ""))
            value = params.get("value", {})
            kind = value.get("kind")
            title = value.get("title") or value.get("message") or token
            self._saw_progress = True
            self._last_progress = time.monotonic()
            if kind == "begin":
                self._progress_tokens.add(token)
            elif kind == "end":
                self._progress_tokens.discard(token)
            if self._options.verbose and kind:
                print(f"lsp progress: {kind} {title}", file=sys.stderr)
            return
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
            try:
                relative_parts = candidate.relative_to(path).parts
            except ValueError:
                relative_parts = candidate.name,
            parts = set(relative_parts)
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


def _line_start_offsets(text: str) -> list[int]:
    offsets = [0]
    for idx, char in enumerate(text):
        if char == "\n":
            offsets.append(idx + 1)
    return offsets


def _trim_rust_header(header: str) -> str:
    lines = []
    for line in header.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("///"):
            continue
        if stripped.startswith("#["):
            if stripped.startswith("#[cfg") or stripped.startswith("#![cfg"):
                lines.append(stripped)
            continue
        if stripped.startswith("//"):
            continue
        lines.append(stripped)
    return " ".join(lines)


def _enclosing_block_headers(text: str, byte_offset: int) -> list[str]:
    stack: list[tuple[int, str]] = []
    last_boundary = 0
    idx = 0
    paren_depth = 0
    bracket_depth = 0
    angle_depth = 0
    while idx < byte_offset and idx < len(text):
        char = text[idx]
        next_char = text[idx + 1] if idx + 1 < len(text) else ""

        if char == "/" and next_char == "/":
            newline = text.find("\n", idx + 2)
            if newline == -1 or newline >= byte_offset:
                break
            idx = newline + 1
            continue
        if char == "/" and next_char == "*":
            end = text.find("*/", idx + 2)
            if end == -1 or end >= byte_offset:
                break
            idx = end + 2
            continue
        if char == '"':
            idx += 1
            while idx < byte_offset and idx < len(text):
                if text[idx] == "\\":
                    idx += 2
                    continue
                if text[idx] == '"':
                    idx += 1
                    break
                idx += 1
            continue

        if char == "(":
            paren_depth += 1
        elif char == ")" and paren_depth:
            paren_depth -= 1
        elif char == "[":
            bracket_depth += 1
        elif char == "]" and bracket_depth:
            bracket_depth -= 1
        elif char == "<":
            angle_depth += 1
        elif char == ">" and angle_depth:
            angle_depth -= 1
        elif char == "{":
            stack.append((idx, _trim_rust_header(text[last_boundary:idx])))
            last_boundary = idx + 1
        elif char == "}":
            if stack:
                stack.pop()
            last_boundary = idx + 1
        elif char == ";" and paren_depth == 0 and bracket_depth == 0 and angle_depth == 0:
            last_boundary = idx + 1
        idx += 1

    return [header for _, header in stack]


def is_trait_contract_item(path: Path, zero_based_line: int) -> bool:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return False

    offsets = _line_start_offsets(text)
    if zero_based_line >= len(offsets):
        return False

    for header in reversed(_enclosing_block_headers(text, offsets[zero_based_line])):
        if re.search(r"\btrait\s+[A-Za-z_][A-Za-z0-9_]*\b", header):
            return True
        if re.search(r"\bimpl\b", header) and re.search(r"\bfor\b", header):
            return True
    return False


def _leading_attributes(path: Path, zero_based_line: int) -> list[str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return []

    attrs: list[str] = []
    idx = zero_based_line - 1
    while idx >= 0:
        line = lines[idx].strip()
        if not line or line.startswith("///") or line.startswith("//"):
            idx -= 1
            continue
        if line.startswith("#["):
            attrs.append(line)
            idx -= 1
            continue
        break
    return list(reversed(attrs))


def is_runtime_discovered_entrypoint(item: Item) -> bool:
    if item.name == "main":
        return True
    attrs = _leading_attributes(item.path, item.line)
    return any(
        attr.startswith("#[no_mangle")
        or attr.startswith("#[export_name")
        or "wasm_bindgen" in attr
        for attr in attrs
    )


def is_cfg_gated_item(path: Path, zero_based_line: int) -> bool:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return False

    offsets = _line_start_offsets(text)
    if zero_based_line >= len(offsets):
        return False

    if any(attr.startswith("#[cfg") for attr in _leading_attributes(path, zero_based_line)):
        return True
    return any(
        "#[cfg" in header
        for header in _enclosing_block_headers(text, offsets[zero_based_line])
    )


def is_macro_discovered_helper(item: Item) -> bool:
    # Serde discovers these by name from `#[serde(with = "...")]` modules,
    # and Serialize/Deserialize impls use the same names. rust-analyzer
    # reference queries do not see those macro-generated call sites.
    return item.name in {"serialize", "deserialize"}


def has_obvious_text_reference(item: Item) -> bool:
    try:
        text = item.path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return False

    lines = text.splitlines()
    if item.line < len(lines):
        definition_line = lines[item.line]
        text_without_definition = text.replace(definition_line, "", 1)
    else:
        text_without_definition = text

    name = re.escape(item.name)
    call_or_attr = re.compile(
        rf"(\.|::|\b){name}\s*\(|[\"']{name}[\"']",
        re.MULTILINE,
    )
    return call_or_attr.search(text_without_definition) is not None


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
        default=0.0,
        help="Extra seconds to drain rust-analyzer notifications after workspace progress ends.",
    )
    parser.add_argument(
        "--workspace-ready-timeout",
        type=float,
        default=120.0,
        help="Seconds to wait for rust-analyzer work-done progress to become idle.",
    )
    parser.add_argument(
        "--workspace-ready-idle",
        type=float,
        default=1.0,
        help="Idle seconds with no active rust-analyzer progress before scanning.",
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
    parser.add_argument(
        "--output",
        type=Path,
        help="Write the report to this file instead of stdout.",
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
    output_context = (
        args.output.open("w", encoding="utf-8")
        if args.output
        else contextlib.nullcontext(sys.stdout)
    )
    try:
        client.request(
            "initialize",
            {
                "processId": os.getpid(),
                "rootUri": uri(root),
                "capabilities": {
                    "workspace": {"configuration": True},
                    "window": {"workDoneProgress": True},
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
        if args.workspace_ready_timeout > 0:
            client.wait_for_work_done_progress(
                args.workspace_ready_timeout,
                args.workspace_ready_idle,
            )
        client.drain_for(args.startup_delay)

        all_items_count = 0
        findings: list[Finding] = []
        with output_context as output:
            if args.output and not args.json:
                print("# Unreferenced Rust Functions/Methods", file=output)
                print("", file=output)
                print(
                    "Generated by scripts/find_unreferenced_rust_items.py.",
                    file=output,
                )
                print("This is a review queue, not a deletion list.", file=output)
                print("", file=output)

            for file_index, path in enumerate(rust_files, start=1):
                if args.verbose:
                    display_path = path.relative_to(root)
                    print(
                        f"scanning {file_index}/{len(rust_files)}: {display_path}",
                        file=sys.stderr,
                    )
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
                items = flatten_symbols(path, symbols or [])
                all_items_count += len(items)

                for item in items:
                    if not args.include_tests and has_test_attr(item.path, item.line):
                        continue
                    if is_trait_contract_item(item.path, item.line):
                        continue
                    if is_runtime_discovered_entrypoint(item):
                        continue
                    if is_cfg_gated_item(item.path, item.line):
                        continue
                    if is_macro_discovered_helper(item):
                        continue
                    if has_obvious_text_reference(item):
                        continue
                    refs = request_with_content_retry(
                        client,
                        "textDocument/references",
                        {
                            "textDocument": {"uri": uri(item.path)},
                            "position": {
                                "line": item.line,
                                "character": item.character,
                            },
                            "context": {"includeDeclaration": True},
                        },
                        args.request_timeout,
                    )
                    if any(not is_definition_reference(ref, item) for ref in refs or []):
                        continue

                    display_path = item.path.relative_to(root)
                    finding = Finding(
                        path=str(display_path),
                        line=item.line + 1,
                        column=item.character + 1,
                        kind=SYMBOL_KIND_NAMES[item.kind],
                        name=item.name,
                        source=source_line(item.path, item.line),
                    )
                    findings.append(finding)
                    if not args.json:
                        print(finding.format(), file=output)
                        output.flush()

                client.notify(
                    "textDocument/didClose",
                    {"textDocument": {"uri": uri(path)}},
                )

            if all_items_count == 0 and rust_files:
                raise RuntimeError(
                    "rust-analyzer returned zero document symbols for every scanned file; "
                    "the workspace was probably queried before it was ready"
                )

            if args.json:
                print(json.dumps([finding.as_dict() for finding in findings], indent=2), file=output)
            else:
                print(
                    f"\nscanned {all_items_count} functions/methods in "
                    f"{len(rust_files)} files; found {len(findings)} with no references",
                    file=output,
                )

        if args.output:
            print(f"wrote report to {args.output}")
    finally:
        client.close()

    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
