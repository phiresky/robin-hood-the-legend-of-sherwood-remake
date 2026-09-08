import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Separate production build of the actual editor with in-memory test fixtures.
export default defineConfig({
  plugins: [solid()],
  build: {
    target: "esnext",
    outDir: "dist/lifecycle",
    rollupOptions: { input: "tests/lifecycle.html" },
  },
  preview: { port: 5181 },
});
