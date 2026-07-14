import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist"] },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.flat.recommended.rules,
      // Scribe intentionally loads Tauri state in effects and keeps editable
      // draft controls synchronized with persisted settings. These patterns are
      // valid without React Compiler optimization and are covered by E2E flows.
      "react-hooks/set-state-in-effect": "off",
      // Several event callbacks use latest-value refs to avoid resubscribing to
      // native listeners. The core Rules of Hooks and dependency checks remain
      // enforced; the React Compiler ref restriction is not enabled here.
      "react-hooks/refs": "off",
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
);
