import { readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
const hash = (b) => createHash("sha256").update(b).digest("hex");
/** Read-only feed over a producer-owned append-only spool; no game command forwarding. */
export async function createObservationBridge(directory) {
  const root = await realpath(directory);
  async function read(name) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*\.json$/.test(name))
      throw Error("Unsafe observation filename");
    const p = await realpath(path.join(root, name));
    if (path.dirname(p) !== root) throw Error("Observation path escaped spool");
    return readFile(p);
  }
  const manifestBytes = await read("session.json"),
    manifest = JSON.parse(manifestBytes),
    pin = hash(manifestBytes);
  let retired = false,
    lastCount = 0;
  if (manifest.kind !== "live-state-observations" || !/^[a-f0-9]{32}$/.test(manifest.session_id))
    throw Error("Invalid observation session");
  async function checked(row) {
    const b = await read(row.file);
    if (hash(b) !== row.sha256) throw Error("Observation resource changed");
    return b;
  }
  return async function observationRequest(request, response, next) {
    const prefix = "/runtime-observations/";
    if (!request.url?.startsWith(prefix)) return next();
    const host = request.headers.host;
    if (
      !/^(127\.0\.0\.1|localhost)(:[0-9]+)?$/.test(host ?? "") ||
      (request.headers.origin && request.headers.origin !== `http://${host}`) ||
      (request.headers["sec-fetch-site"] &&
        !["same-origin", "none"].includes(request.headers["sec-fetch-site"]))
    ) {
      response.writeHead(403).end();
      return;
    }
    if (request.method !== "GET") {
      response.writeHead(405).end();
      return;
    }
    try {
      if (retired || hash(await read("session.json")) !== pin)
        throw Error("Observation session retired or replaced");
      const route = request.url.slice(prefix.length);
      let bytes;
      if (route === "session") bytes = manifestBytes;
      else if (route === "assets") bytes = await checked(manifest.assets);
      else if (route === "qualification") bytes = await checked(manifest.authority.receipt);
      else if (/^packet\/(0|[1-9][0-9]{0,9})$/.test(route)) {
        const sequence = Number(route.slice(7)),
          head = JSON.parse(await read("head.json"));
        if (
          head.session_id !== manifest.session_id ||
          !Number.isSafeInteger(head.count) ||
          head.count < lastCount ||
          typeof head.closed !== "boolean"
        )
          throw Error("Observation producer restarted");
        if (head.error) throw Error(`Observation producer failed: ${String(head.error)}`);
        lastCount = head.count;
        if (sequence > head.count) throw Error("Observation sequence gap");
        if (sequence === head.count) {
          if (!head.closed) {
            response.writeHead(204, { "Cache-Control": "no-store" }).end();
            return;
          }
          bytes = Buffer.from(
            JSON.stringify({ session_id: manifest.session_id, sequence, closed: true }),
          );
        } else {
          const packet = JSON.parse(await read(`packet-${sequence}.json`));
          if (
            packet.session_id !== manifest.session_id ||
            packet.sequence !== sequence ||
            packet.engine_frame !== manifest.first_engine_frame + sequence
          )
            throw Error("Observation packet identity or frame gap");
          const engine = await checked(packet.engine);
          bytes = Buffer.from(
            JSON.stringify({
              session_id: packet.session_id,
              sequence,
              engine_frame: packet.engine_frame,
              engine_sha256: packet.engine.sha256,
              engine_text: engine.toString("utf8"),
            }),
          );
        }
      } else {
        response.writeHead(404).end();
        return;
      }
      response.writeHead(200, {
        "Content-Type": "application/json",
        "Cache-Control": "no-store",
        "X-Content-Type-Options": "nosniff",
      });
      response.end(bytes);
    } catch (error) {
      retired = true;
      response
        .writeHead(409, { "Content-Type": "application/json", "Cache-Control": "no-store" })
        .end(JSON.stringify({ error: String(error) }));
    }
  };
}
export function observationBridgePlugin(directory) {
  const mount = (server) => {
    if (!directory) return;
    const ready = createObservationBridge(directory).then(
      (handler) => ({ handler }),
      (error) => ({ error }),
    );
    server.middlewares.use((req, res, next) => {
      if (!req.url?.startsWith("/runtime-observations/")) return next();
      void ready
        .then((result) => {
          if (result.error) {
            res.writeHead(503).end(String(result.error));
            return;
          }
          return result.handler(req, res, next);
        })
        .catch((error) => res.writeHead(503).end(String(error)));
    });
  };
  return {
    name: "read-only-runtime-observations",
    configureServer: mount,
    configurePreviewServer: mount,
  };
}
