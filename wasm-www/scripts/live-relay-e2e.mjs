#!/usr/bin/env node

/**
 * Drive a native-hosted browser multiplayer session through Chrome CDP.
 *
 * Chrome must already be listening on `--remote-debugging-port` with an
 * about:blank page. The native host must expose its loopback script API so
 * the driver can prove that browser RPC input reached authoritative state.
 */

const options = parseArgs(process.argv.slice(2));
const ticket = required(options, "ticket");
const devtoolsPort = Number(options["devtools-port"] ?? 9222);
const hostHttp = options["host-http"] ?? "http://127.0.0.1:18641";
const pageUrl = new URL(options["page-url"] ?? "http://127.0.0.1:41739/");
pageUrl.searchParams.set("wasm-base", options["wasm-base"] ?? "/wasm/local");
pageUrl.searchParams.set("join", ticket);
pageUrl.searchParams.set("mp-nickname", options.nickname ?? "e2e-browser");
pageUrl.searchParams.set("no-sound", "true");
pageUrl.searchParams.set("rollback-check", "false");
pageUrl.searchParams.set("wasm-log", options["wasm-log"] ?? "info");

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const pages = await fetch(`http://127.0.0.1:${devtoolsPort}/json/list`).then(response => {
    if (!response.ok) {
        throw new Error(`Chrome target list returned HTTP ${response.status}`);
    }
    return response.json();
});
const page = pages.find(candidate => candidate.type === "page") ?? pages[0];
if (page === undefined) {
    throw new Error("Chrome has no page target");
}

const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
});

let nextId = 1;
const pending = new Map();
const logLines = [];

socket.addEventListener("message", event => {
    const message = JSON.parse(event.data);
    if (message.id !== undefined) {
        const waiter = pending.get(message.id);
        if (waiter !== undefined) {
            pending.delete(message.id);
            if (message.error !== undefined) {
                waiter.reject(new Error(message.error.message));
            } else {
                waiter.resolve(message.result ?? {});
            }
        }
        return;
    }

    if (message.method === "Runtime.consoleAPICalled") {
        const line = message.params.args.map(consoleArgText).join(" ");
        logLines.push(line);
        if (/multiplayer|relay|snapshot|begin-sim|panicked|boot failed/i.test(line)) {
            console.log(`BROWSER_LOG ${line}`);
        }
    } else if (message.method === "Runtime.exceptionThrown") {
        const exception = message.params.exceptionDetails.exception?.description ?? "";
        const line = `${message.params.exceptionDetails.text} ${exception}`;
        logLines.push(line);
        console.log(`BROWSER_EXCEPTION ${line}`);
    } else if (message.method === "Log.entryAdded") {
        const line = message.params.entry.text;
        logLines.push(line);
        if (/error|warn|websocket|wasm/i.test(line)) {
            console.log(`CHROME_LOG ${line}`);
        }
    } else if (message.method === "Network.webSocketCreated") {
        console.log(`WEBSOCKET_CREATED ${message.params.url}`);
    } else if (message.method === "Network.webSocketClosed") {
        console.log(`WEBSOCKET_CLOSED ${message.params.requestId}`);
    }
});

function send(method, params = {}) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.send(JSON.stringify({ id, method, params }));
    });
}

async function evaluate(expression) {
    const response = await send("Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
    });
    if (response.exceptionDetails !== undefined) {
        const exception = response.exceptionDetails.exception?.description ?? "";
        throw new Error(`${response.exceptionDetails.text} ${exception}`);
    }
    return response.result?.value;
}

function rpc(method, params = null) {
    return evaluate(
        `globalThis.robinRpc(${JSON.stringify(method)},${JSON.stringify(params)})`,
    );
}

async function waitForLog(fragment, timeoutMilliseconds, startIndex = 0) {
    const started = Date.now();
    for (;;) {
        const match = logLines.slice(startIndex).find(line => line.includes(fragment));
        if (match !== undefined) {
            return match;
        }
        if (Date.now() - started > timeoutMilliseconds) {
            throw new Error(`timed out waiting for browser log: ${fragment}`);
        }
        await sleep(100);
    }
}

function collectLockFlags(value, output = []) {
    if (Array.isArray(value)) {
        for (const child of value) {
            collectLockFlags(child, output);
        }
    } else if (value !== null && typeof value === "object") {
        if (Object.hasOwn(value, "is_lock_alt")) {
            output.push(value.is_lock_alt);
        }
        for (const child of Object.values(value)) {
            collectLockFlags(child, output);
        }
    }
    return output;
}

async function hostLockFlags() {
    const dump = await fetch(`${hostHttp}/engine-dump`).then(response => {
        if (!response.ok) {
            throw new Error(`host engine dump returned HTTP ${response.status}`);
        }
        return response.json();
    });
    return collectLockFlags(dump);
}

async function waitForHostFlags(expected, timeoutMilliseconds = 30_000) {
    const started = Date.now();
    for (;;) {
        const actual = await hostLockFlags();
        if (JSON.stringify(actual) === JSON.stringify(expected)) {
            return actual;
        }
        if (Date.now() - started > timeoutMilliseconds) {
            throw new Error(
                `host flags remained ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
            );
        }
        await sleep(250);
    }
}

async function stepHost(frames) {
    for (let index = 0; index < frames; index += 1) {
        const response = await fetch(`${hostHttp}/step-forward`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ n: 1, auto_dismiss: false }),
        });
        if (!response.ok) {
            throw new Error(
                `host step request returned HTTP ${response.status}: ${await response.text()}`,
            );
        }
        const result = await response.json();
        console.log(`HOST_STEP ${JSON.stringify(result)}`);
        // A separate outer frame must run between debugger steps so network
        // ingress can publish commands whose target frame just became due.
        await sleep(100);
    }
}

try {
    await send("Runtime.enable");
    await send("Log.enable");
    await send("Network.enable");
    await send("Page.enable");
    await send("Page.addScriptToEvaluateOnNewDocument", {
        source: `(() => {
            const NativeWebSocket = globalThis.WebSocket;
            const sockets = [];
            Object.defineProperty(globalThis, "__robinE2ESockets", { value: sockets });
            globalThis.WebSocket = new Proxy(NativeWebSocket, {
                construct(target, args, newTarget) {
                    const created = Reflect.construct(target, args, newTarget);
                    sockets.push(created);
                    return created;
                },
            });
        })();`,
    });

    const redactedUrl = new URL(pageUrl);
    redactedUrl.searchParams.set("join", "rhmp1-<ticket>");
    console.log(`NAVIGATE ${redactedUrl}`);
    // Runtime.enable may replay console entries from a previous page in a
    // reused Chrome target. Only protocol events from this navigation count.
    logLines.length = 0;
    await send("Page.navigate", { url: pageUrl.toString() });

    await waitForLog("browser welcomed through iroh WebSocket relay", 180_000);
    console.log("PROTOCOL_EVENT browser welcomed through iroh WebSocket relay");
    await waitForLog("frame-0 host snapshot", 300_000);
    await waitForLog("multiplayer: begin-sim barrier released", 180_000);
    console.log(`INITIAL_STATE ${JSON.stringify(await rpc("state"))}`);

    await stepHost(4);
    console.log(`INITIAL_HOST_LOCK_FLAGS ${JSON.stringify(await waitForHostFlags([false, false]))}`);
    console.log(`FIRST_RPC ${JSON.stringify(await rpc("command", { SetLockAlt: true }))}`);
    await stepHost(4);
    console.log(
        `HOST_LOCK_FLAGS_AFTER_FIRST_INPUT ${JSON.stringify(await waitForHostFlags([false, true]))}`,
    );
    try {
        const replayReply = await rpc("get-replay");
        const replay =
            typeof replayReply === "string" ? replayReply : String(replayReply?.content ?? "");
        console.log(`REPLAY_AFTER_FIRST_INPUT prefix=${replay.slice(0, 18)} length=${replay.length}`);
    } catch (error) {
        console.log(`REPLAY_AFTER_FIRST_INPUT unavailable=${error.message}`);
    }

    const socketExpression =
        "globalThis.__robinE2ESockets.map((socket, index) => ({ index, url: socket.url, state: socket.readyState }))";
    console.log(`SOCKETS_BEFORE_DROP ${JSON.stringify(await evaluate(socketExpression))}`);
    const reconnectLogStart = logLines.length;
    const closed = await evaluate(`globalThis.__robinE2ESockets
        .filter(socket => socket.readyState === WebSocket.OPEN && !socket.url.includes("127.0.0.1:41739"))
        .map(socket => { const url = socket.url; socket.close(4001, "relay e2e disconnect"); return url; })`);
    console.log(`FORCED_RELAY_SOCKET_CLOSE ${JSON.stringify(closed)}`);

    try {
        await waitForLog(
            "browser multiplayer session ended; reconnecting through iroh relay",
            45_000,
            reconnectLogStart,
        );
    } catch {
        console.log("SOCKET_CLOSE_FALLBACK Network.emulateNetworkConditions offline");
        await send("Network.emulateNetworkConditions", {
            offline: true,
            latency: 0,
            downloadThroughput: 0,
            uploadThroughput: 0,
        });
        // Stay offline beyond QUIC's idle window. A shorter outage merely
        // makes iroh replace the relay WebSocket under the live session and
        // does not exercise the game's full reconnect/snapshot path.
        await sleep(45_000);
        await send("Network.emulateNetworkConditions", {
            offline: false,
            latency: 0,
            downloadThroughput: -1,
            uploadThroughput: -1,
        });
        await waitForLog(
            "browser multiplayer session ended; reconnecting through iroh relay",
            45_000,
            reconnectLogStart,
        );
    }

    await waitForLog(
        "multiplayer: transport reconnected; awaiting host snapshot",
        180_000,
        reconnectLogStart,
    );
    await waitForLog("multiplayer: adopt", 180_000, reconnectLogStart);
    console.log(`RECONNECTED_STATE ${JSON.stringify(await rpc("state"))}`);
    console.log(
        `HOST_LOCK_FLAGS_AFTER_SNAPSHOT ${JSON.stringify(await waitForHostFlags([false, true]))}`,
    );

    console.log(`SECOND_RPC ${JSON.stringify(await rpc("command", { SetLockAlt: false }))}`);
    await stepHost(4);
    console.log(
        `HOST_LOCK_FLAGS_AFTER_SECOND_INPUT ${JSON.stringify(await waitForHostFlags([false, false]))}`,
    );
    try {
        const replayReply = await rpc("get-replay");
        const replay =
            typeof replayReply === "string" ? replayReply : String(replayReply?.content ?? "");
        console.log(`REPLAY_FINAL prefix=${replay.slice(0, 18)} length=${replay.length}`);
    } catch (error) {
        console.log(`REPLAY_FINAL unavailable=${error.message}`);
    }
    console.log(`SOCKETS_FINAL ${JSON.stringify(await evaluate(socketExpression))}`);
    console.log("E2E_COMPLETE");
    await send("Browser.close").catch(() => {});
} catch (error) {
    console.error(`E2E_FAILURE ${error.stack ?? error.message}`);
    await send("Browser.close").catch(() => {});
    process.exitCode = 1;
} finally {
    await sleep(250);
    socket.close();
}

function consoleArgText(arg) {
    if (Object.hasOwn(arg, "value")) {
        return typeof arg.value === "string" ? arg.value : JSON.stringify(arg.value);
    }
    return arg.description ?? arg.unserializableValue ?? arg.type ?? "";
}

function parseArgs(args) {
    const parsed = {};
    for (let index = 0; index < args.length; index += 2) {
        const key = args[index];
        const value = args[index + 1];
        if (key === undefined || !key.startsWith("--") || value === undefined) {
            throw new Error(`expected --name value arguments, got ${args.slice(index).join(" ")}`);
        }
        parsed[key.slice(2)] = value;
    }
    return parsed;
}

function required(values, key) {
    const value = values[key];
    if (value === undefined || value.length === 0) {
        throw new Error(`missing required --${key}`);
    }
    return value;
}
