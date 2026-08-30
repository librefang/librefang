A GitHub Release stays a draft until its artifacts are all present and signed.
It used to be published before a single job had run, so any failure left it permanently public, incomplete and unsigned — v2026.8.30 shipped that way with 23 of 48 assets and no `SHA256SUMS`, while the job that refuses to sign an incomplete platform set failed to no effect, because nothing depended on it.
Publication now waits on the signature and the desktop bundles, and the two formulae that bake a public download URL wait on publication; best-effort mobile jobs and the package-registry mirrors deliberately do not gate it.
(#8009) (@houko)
