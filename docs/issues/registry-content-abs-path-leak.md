# [Low] API surface Low — registry abs path leak, `validate_template_name` lift, NUL accept, CSP `unsafe-inline`

**Severity:** Low · **Domain:** API attack surface
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `register_registry_content` returns an absolute path, leaking the home-directory structure | `routes/registry.rs:276-281` |
| validate_template_name lift | `validate_template_name` is copy-pasted in several places; should be lifted to a shared helper | multiple sites |
| NUL accept | The dashboard path extractor accepts NUL (`\0`) characters | dashboard path extraction |
| CSP unsafe-inline | CSP contains `'unsafe-inline'` in `style-src` — a hole in the second XSS defense line | response header injection site |

## Combined fix plan

1. (this) Return a path relative to `home_dir`, or just the resource name.
2. (validate_template_name lift) `pub fn validate_template_name(s: &str) -> Result<(), _>` in a shared crate; migrate all callers and delete the copies.
3. (NUL accept) The path extractor rejects any input containing `\0`; reject with 400.
4. (CSP unsafe-inline) Remove `'unsafe-inline'` from `style-src`; switch styles that must be inline to `style-src 'nonce-XXX'`.
