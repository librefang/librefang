import { describe, it, expect, beforeAll } from "vitest";
import { render, screen } from "@testing-library/react";
import i18n from "i18next";
import { I18nextProvider, Trans } from "react-i18next";
import { initReactI18next } from "react-i18next";

// Regression test for the rollup item in
// docs/issues/i18n-escapeValue-false.md ("i18n escapeValue: false +
// dangerouslySetInnerHTML XSS landmine"). Before the fix, every value
// interpolated through `t(...)` was inserted into the resulting string
// without HTML escaping; consumers (MobilePairingPage, ConnectWizardPage)
// then handed those strings to `dangerouslySetInnerHTML`. A translator-PR
// that smuggled `<script>` into a locale file would execute in the
// dashboard origin. We now (1) keep i18next's default escaping
// (escapeValue: true) and (2) render any HTML-bearing translation via
// `<Trans>` with an explicit `components` allowlist.

beforeAll(async () => {
  // Use an isolated i18next instance so this test does not race the
  // browser-only LanguageDetector path in `lib/i18n.ts`.
  await i18n.use(initReactI18next).init({
    lng: "en",
    fallbackLng: "en",
    resources: {
      en: {
        translation: {
          attack: {
            // Simulates a malicious / careless translator submission.
            payload: "hello {{user}} <script>alert(1)</script>",
          },
          // Mirrors `mobile_pairing.error_disabled_body` shape — uses a
          // non-void HTML tag (<a>) so html-parse-stringify treats it as a
          // container, not a self-closing void element like <link>.
          link_body: "Enable pairing in <a>Config</a>.",
        },
      },
    },
    interpolation: { escapeValue: true },
  });
});

describe("i18n escape configuration", () => {
  it("escapes HTML in interpolated values so a translator payload cannot inject DOM", () => {
    render(
      <I18nextProvider i18n={i18n}>
        <p data-testid="payload">{i18n.t("attack.payload", { user: "<img onerror=x>" })}</p>
      </I18nextProvider>,
    );
    const el = screen.getByTestId("payload");
    // The crucial property is structural, not textual: nothing inside this
    // paragraph is a live DOM node. The interpolated `<img onerror=x>` is
    // entity-encoded by i18next (escapeValue: true) before reaching React;
    // the literal `<script>` from the translation source is then placed by
    // React as a text node. Neither can execute.
    expect(el.querySelector("script")).toBeNull();
    expect(el.querySelector("img")).toBeNull();
    // No raw `<script` / `<img` tags ever appear in the rendered HTML.
    expect(el.innerHTML).not.toContain("<script");
    expect(el.innerHTML).not.toContain("<img");
  });

  it("<Trans> only renders the components listed in its allowlist", () => {
    render(
      <I18nextProvider i18n={i18n}>
        <p data-testid="link-body">
          <Trans
            i18n={i18n}
            i18nKey="link_body"
            components={{ a: <a href="/dashboard/config/security" /> }}
          />
        </p>
      </I18nextProvider>,
    );
    const el = screen.getByTestId("link-body");
    const anchor = el.querySelector("a");
    expect(anchor).not.toBeNull();
    expect(anchor?.getAttribute("href")).toBe("/dashboard/config/security");
    expect(anchor?.textContent).toBe("Config");
  });
});
