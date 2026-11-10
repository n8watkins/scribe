import { searchForWorkspaceRoot } from "vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  server: {
    fs: {
      // Same as vite.config.ts: the app bundles the repo root CHANGELOG.md.
      // @ts-expect-error process is a nodejs global
      allow: [searchForWorkspaceRoot(process.cwd()), ".."],
    },
  },
  test: {
    // Stylesheets stay stubbed out, except `?raw` imports: tests read those
    // as text to check style contracts (see scrollbarGutter.test.ts).
    css: { include: [/\.css\?raw$/] },
    environment: "jsdom",
  },
});
