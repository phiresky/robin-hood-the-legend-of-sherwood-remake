// Long bake comparisons must not reload when another task edits the editor.
import path from "node:path";
import { createRequire } from "node:module";
import config from "../app/vite.config.ts";
const { createServer } = await import(
  createRequire(new URL("../app/package.json", import.meta.url)).resolve("vite")
);
const port = Number(process.argv[2] ?? 5191);
if (!Number.isSafeInteger(port) || port < 1024 || port > 65535)
  throw new Error("Invalid fixture port");
const server = await createServer({
  ...config,
  configFile: false,
  root: path.resolve("app"),
  server: { ...config.server, host: "127.0.0.1", port, strictPort: true, hmr: false, watch: null },
});
await server.listen();
server.printUrls();
console.log("Fixture file watching and hot reload disabled");
