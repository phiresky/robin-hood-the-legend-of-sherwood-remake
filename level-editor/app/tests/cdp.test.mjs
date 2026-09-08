import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromeEndpoint, socketOpen, evaluate } from "./cdp.mjs";

class Socket extends EventTarget {
  readyState = 0;
  active = new Set();
  addEventListener(type, callback, options) {
    this.active.add(callback);
    super.addEventListener(type, callback, options);
  }
  removeEventListener(type, callback, options) {
    this.active.delete(callback);
    super.removeEventListener(type, callback, options);
  }
  send() {}
}

test("startup handles split endpoint output and cleans listeners on failure", async () => {
  for (const action of ["endpoint", "error", "exit", "timeout"]) {
    const child = new EventEmitter();
    child.stderr = new EventEmitter();
    const pending = chromeEndpoint(child, { timeoutMs: 20 });
    if (action === "endpoint") {
      child.stderr.emit("data", "DevTools listening on ws://127.0.");
      child.stderr.emit("data", "0.1:9000/browser/id\n");
      assert.equal(await pending, "ws://127.0.0.1:9000/browser/id");
    } else {
      if (action === "error") child.emit("error", new Error("spawn failed"));
      if (action === "exit") child.emit("exit", 2);
      await assert.rejects(pending);
    }
    assert.equal(child.listenerCount("error"), 0);
    assert.equal(child.listenerCount("exit"), 0);
    assert.equal(child.stderr.listenerCount("data"), 0);
  }
});

test("socket open rejects disconnect, transport failure, abort and timeout", async () => {
  for (const action of ["close", "error", "abort", "timeout"]) {
    const socket = new Socket();
    const controller = new AbortController();
    const pending = socketOpen(socket, {
      signal: controller.signal,
      timeoutMs: 20,
    });
    if (action === "abort") controller.abort(new Error("browser exited"));
    else if (action !== "timeout") socket.dispatchEvent(new Event(action));
    await assert.rejects(pending);
    assert.equal(socket.active.size, 0);
  }
});

test("CDP requests reject failure and always remove response subscriptions", async () => {
  for (const action of [
    "close",
    "error",
    "abort",
    "timeout",
    "protocol",
    "exception",
    "send",
    "success",
  ]) {
    const socket = new Socket();
    socket.readyState = 1;
    const controller = new AbortController();
    if (action === "send")
      socket.send = () => {
        throw new Error("socket disappeared");
      };
    const pending = evaluate(socket, 42, "1", {
      signal: controller.signal,
      timeoutMs: 20,
    });
    if (action === "abort") controller.abort(new Error("browser exited"));
    if (action === "close" || action === "error")
      socket.dispatchEvent(new Event(action));
    if (["protocol", "exception", "success"].includes(action))
      socket.dispatchEvent(
        new MessageEvent("message", {
          data: JSON.stringify({
            id: 42,
            error:
              action === "protocol" ? { message: "bad request" } : undefined,
            result:
              action === "exception"
                ? { exceptionDetails: { text: "failed" } }
                : { result: { value: 1 } },
          }),
        }),
      );
    if (action === "success") assert.equal(await pending, 1);
    else await assert.rejects(pending);
    assert.equal(socket.active.size, 0);
  }
});

test("failed Chrome spawn exits promptly and removes only its owned profile", async () => {
  const directory = await mkdtemp(join(tmpdir(), "editor-cdp-test-"));
  try {
    await assert.rejects(
      promisify(execFile)(
        process.execPath,
        [fileURLToPath(new URL("./run-lifecycle.mjs", import.meta.url))],
        {
          env: {
            ...process.env,
            CHROME: join(directory, "missing-chrome"),
            TMPDIR: directory,
          },
          timeout: 5000,
        },
      ),
      (error) => error.code === 1 && error.stderr.includes("ENOENT"),
    );
    assert.deepEqual(await readdir(directory), []);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
