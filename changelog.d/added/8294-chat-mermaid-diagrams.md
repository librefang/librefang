Mermaid diagrams in a chat message are now drawn instead of shown as their source.
Agents answer with Mermaid often — an architecture sketch, a sequence diagram for a flow they just traced — and until now you read `graph TD; A-->B;` and had to picture it yourself.
A diagram that does not render still gives you its source back rather than an error, and the source stays one click away under every diagram that does.
Diagrams are sized to sit inside the conversation rather than to set its width, and a tall one is capped to a thumbnail instead of taking a screen to itself.
Clicking any diagram opens it at full size in a view that scrolls both ways, which is where a dense one is actually readable.
That cap is not only cosmetic: Mermaid writes its own width onto the SVG, and a diagram whose source asks for a fixed pixel width would otherwise decide how wide the conversation is.
Nothing is downloaded until a diagram actually appears: the library and its layout engine are split out of the main bundle, and a fence only becomes a diagram once the turn has settled, so a half-written one reads as text while it streams.
Mermaid is pinned to an exact version rather than a range, because its own sanitizer is the whole defence between a diagram's source — model output, and so untrusted — and the dashboard, and the hostile cases were verified against that one version; moving it is a place to re-run those checks rather than something a minor bump does quietly.
Mermaid brings `lodash-es` in on the 4.17 line, which carries a high-severity code injection through `_.template` (CVE-2026-4800) and a prototype pollution in `_.unset` / `_.omit` (CVE-2026-2950); a resolution override moves the whole subtree onto the patched 4.18 line. (#8294) (@DaBlitzStein)
