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
    environment: "jsdom",
  },
});
