// Flat-config ESLint for the LibreFang dashboard.
//
// Motivation (refs #5561): provide a CI guard for security-sensitive JSX
// patterns — primarily `target="_blank"` without `rel="noopener noreferrer"`
// (the bug class #5390 cleaned up by hand) and `dangerouslySetInnerHTML`
// passed alongside `children` (silent prop loss + XSS surface).
//
// The baseline is intentionally minimal so the initial bootstrap doesn't
// drown the dashboard in low-signal warnings. We enable the canonical
// recommended sets and then promote the two security rules above to
// `error`. Style / preference rules stay at `warn` (or off) until the
// team opts in.

import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactPlugin from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default [
  // Files / dirs that are never linted. Generated code, build output,
  // vendored assets, and the public/ static tree should not be scanned.
  {
    ignores: [
      "node_modules/**",
      "dist/**",
      "build/**",
      "playwright-report/**",
      "test-results/**",
      "openapi/generated.ts",
      "public/**",
      "scripts/**", // .mjs helpers; not worth a separate parser config
      "e2e/**", // Playwright suite; covered by playwright's own runner
      "coverage/**",
    ],
  },

  // Baseline recommended rule sets, applied workspace-wide.
  js.configs.recommended,
  ...tseslint.configs.recommended,

  // React + React Hooks for .ts/.tsx — applied to source only.
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: {
      react: reactPlugin,
      "react-hooks": reactHooks,
    },
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
        ecmaFeatures: { jsx: true },
      },
      globals: {
        ...globals.browser,
        ...globals.es2021,
      },
    },
    settings: {
      react: { version: "detect" },
    },
    rules: {
      // Pull in the recommended React + Hooks rules without spreading
      // entire configs (flat-config equivalents).
      ...reactPlugin.configs.recommended.rules,
      ...reactHooks.configs.recommended.rules,

      // ── Security guards — the motivating rules for #5561 ───────────
      // Block `target="_blank"` without `rel="noopener noreferrer"`.
      // Existing call sites were cleaned up in #5390; this rule keeps
      // future PRs from regressing.
      "react/jsx-no-target-blank": [
        "error",
        { allowReferrer: false, enforceDynamicLinks: "always" },
      ],
      // Reject `dangerouslySetInnerHTML` combined with `children` — the
      // children prop is silently dropped and the combination is almost
      // always a bug.
      "react/no-danger-with-children": "error",

      // ── Pragmatic adjustments for this codebase ────────────────────
      // Vite + automatic JSX runtime: React import is not required.
      "react/react-in-jsx-scope": "off",
      // We rely on TypeScript for prop-shape checking, not PropTypes.
      "react/prop-types": "off",
      // Allow `<a target="_blank" rel="noopener noreferrer">` and other
      // unescaped entity patterns common in copy strings; the security
      // rule above is the load-bearing one.
      "react/no-unescaped-entities": "off",

      // ── TypeScript noise reduction for the initial bootstrap ──────
      // Demoted to `warn` (with `_`-prefix opt-out) so we don't fail
      // CI on legacy untyped helpers. Tighten later in a follow-up.
      "@typescript-eslint/no-unused-vars": [
        "warn",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrorsIgnorePattern: "^_",
        },
      ],
      // The dashboard has a handful of `any`s in legacy helpers; demote
      // to warn so we don't block the bootstrap PR.
      "@typescript-eslint/no-explicit-any": "warn",
      // Triple-slash refs are required for vite-env.d.ts.
      "@typescript-eslint/triple-slash-reference": "off",
      // `{}` / `Function` etc. show up in some upstream types we
      // re-export; downgrade to warn for now.
      "@typescript-eslint/no-empty-object-type": "warn",
      // Empty catch / interface bodies are usually intentional in this
      // codebase (e.g. swallow-and-fallback paths).
      "no-empty": ["warn", { allowEmptyCatch: true }],
      // Stand-alone `case x: { ... }` blocks declare locals; this is
      // already covered by tsc with `noFallthroughCasesInSwitch`.
      "no-case-declarations": "off",

      // ── Demoted-to-warn for the bootstrap PR (follow-up issue) ────
      // These have small, real baselines we want to clean up
      // incrementally rather than block the initial CI gate on.
      //
      //   * `react-hooks/rules-of-hooks` — `ChatPage.tsx` calls hooks
      //     after an early return for `system` messages; needs a real
      //     refactor of MessageBubble to fix correctly. Tracked as a
      //     follow-up; demoted here so the gate ships.
      //   * `no-unused-expressions` — a couple of inline
      //     `cond ? a() : b()` statement shorthands in event handlers
      //     (CanvasPage / TerminalPage). Stylistic; not a defect.
      //   * `no-irregular-whitespace` — `csvParser.ts` deliberately
      //     references unicode whitespace classes inside comments
      //     describing real-world CSV pitfalls.
      //   * `no-control-regex` — `TerminalTabs.tsx` ANSI / xterm
      //     handling legitimately matches control characters.
      "react-hooks/rules-of-hooks": "warn",
      "@typescript-eslint/no-unused-expressions": "warn",
      "no-irregular-whitespace": "warn",
      "no-control-regex": "warn",
    },
  },

  // Test files — relax a couple of rules that are noisy in vitest specs.
  {
    files: [
      "src/**/*.test.{ts,tsx}",
      "src/lib/__tests__/**/*.{ts,tsx}",
      "src/lib/test/**/*.{ts,tsx}",
    ],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
        ...globals.es2021,
      },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-non-null-assertion": "off",
    },
  },

  // Config-style files at the repo root of the dashboard.
  {
    files: ["*.config.{js,ts,mjs}", "vitest.config.ts", "vite.config.ts"],
    languageOptions: {
      globals: {
        ...globals.node,
        ...globals.es2021,
      },
    },
  },
];
