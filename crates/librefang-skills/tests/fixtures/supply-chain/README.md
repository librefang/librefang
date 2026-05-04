# Supply-chain audit fixtures

These files are **intentionally malicious** examples used to verify that
`scripts/check-skills-supply-chain.py` catches the patterns it claims to
catch. The directory `tests/fixtures/supply-chain/` is on the audit
script's default exclude list, so the live CI scan never reads them — the
self-test job (`--self-test`) reads the equivalent fixtures from an
embedded in-script copy and asserts they all trip the expected rule.

Do not import, install, or `cargo build`-into-binary anything in this
folder. They exist as documentation of the threat shapes and as a
target for `grep`-based audits when adjusting the rule set.

`supply-chain-audit: allow` — opt-out marker so the audit script doesn't
flag this README itself for the documentation phrases below.

Threat shapes covered:

- `clean.py` — benign Python; must NOT be flagged.
- `malicious_eval.py` — `eval(base64.b64decode(...).decode())` payload.
- `malicious_syspath.py` — import-path hijack via `sys.path.insert`.
- `jailbreak_prompt.md` — prompt injection / exfiltration phrases.
- `hijack.pth` — Python `.pth` site-packages hook (auto-executed at
  interpreter start when placed under `site-packages/`).
