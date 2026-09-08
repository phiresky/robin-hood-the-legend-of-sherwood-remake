/** Own every timer/listener until completion, including synchronous failure. */
function bounded(label, { signal, timeoutMs = 5000 }, subscribe) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let unsubscribe;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      unsubscribe?.();
      if (error) reject(error);
      else resolve(value);
    };
    const abort = () => finish(signal.reason ?? new Error(`${label} aborted`));
    const timer = setTimeout(
      () => finish(new Error(`${label} timed out`)),
      timeoutMs,
    );
    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    try {
      unsubscribe = subscribe(finish);
      if (settled) unsubscribe?.();
    } catch (error) {
      finish(error);
    }
  });
}

export function chromeEndpoint(chrome, options = {}) {
  return bounded("Chromium startup", options, (finish) => {
    let output = "";
    const data = (chunk) => {
      output = (output + String(chunk)).slice(-8192);
      const match = output.match(/DevTools listening on (ws:\/\/[^\s]+)\s/);
      if (match) finish(null, match[1]);
    };
    const error = (error) => finish(error);
    const exit = (code, signal) =>
      finish(new Error(`Chromium exited during startup (${code ?? signal})`));
    chrome.stderr.on("data", data);
    chrome.on("error", error);
    chrome.on("exit", exit);
    return () => {
      chrome.stderr.off("data", data);
      chrome.off("error", error);
      chrome.off("exit", exit);
    };
  });
}

export function socketOpen(socket, options = {}) {
  return bounded("CDP connection", options, (finish) => {
    if (socket.readyState === 1) {
      finish();
      return;
    }
    if (socket.readyState > 1) {
      finish(new Error("CDP socket already closed"));
      return;
    }
    const open = () => finish();
    const close = () => finish(new Error("CDP disconnected before opening"));
    const error = () => finish(new Error("CDP connection failed"));
    socket.addEventListener("open", open);
    socket.addEventListener("close", close);
    socket.addEventListener("error", error);
    return () => {
      socket.removeEventListener("open", open);
      socket.removeEventListener("close", close);
      socket.removeEventListener("error", error);
    };
  });
}

export function evaluate(socket, request, expression, options = {}) {
  return bounded("CDP evaluation", options, (finish) => {
    if (socket.readyState !== 1) {
      finish(new Error("CDP socket is not open"));
      return;
    }
    const message = (event) => {
      let response;
      try {
        response = JSON.parse(event.data);
      } catch (error) {
        finish(error);
        return;
      }
      if (response.id !== request) return;
      if (response.error || response.result?.exceptionDetails) {
        finish(
          new Error(
            `CDP evaluation failed: ${JSON.stringify(response.error ?? response.result.exceptionDetails)}`,
          ),
        );
      } else finish(null, response.result?.result?.value);
    };
    const close = () => finish(new Error("CDP disconnected during evaluation"));
    const error = () => finish(new Error("CDP evaluation transport failed"));
    socket.addEventListener("message", message);
    socket.addEventListener("close", close);
    socket.addEventListener("error", error);
    // Register cleanup before send can throw synchronously.
    try {
      socket.send(
        JSON.stringify({
          id: request,
          method: "Runtime.evaluate",
          params: { expression, returnByValue: true },
        }),
      );
    } catch (error) {
      finish(error);
    }
    return () => {
      socket.removeEventListener("message", message);
      socket.removeEventListener("close", close);
      socket.removeEventListener("error", error);
    };
  });
}
