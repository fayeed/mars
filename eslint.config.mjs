// @ts-check

import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

export default defineConfig([
  {
    // Fixture repositories are what the indexer reads, not code Mars runs;
    // one is malformed on purpose.
    ignores: [
      "**/dist/**",
      "**/gen/**",
      "**/node_modules/**",
      "target/**",
      "fixtures/repositories/**",
    ],
  },
  {
    files: ["**/*.{js,mjs,ts,tsx}"],
    extends: [js.configs.recommended, tseslint.configs.recommended],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
  {
    ...reactHooks.configs.flat.recommended,
    files: ["apps/desktop/src/**/*.{ts,tsx}"],
  },
]);
