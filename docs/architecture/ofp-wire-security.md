# OFP Wire Security: Plaintext-on-the-Wire by Design

**Status**: documented limitation. Tracked: closed [#3874](https://github.com/librefang/librefang/issues/3874), closed [#4001](https://github.com/librefang/librefang/pull/4001).
**Crate**: `librefang-wire`.

## What OFP guarantees today

OFP — the LibreFang federation protocol that lets one kernel call agents on
another kernel — does **not** encrypt traffic on the wire. It does provide:

- **Mutual authentication** of peers via a pre-shared `shared_secret` and an
  HMAC-SHA256 handshake (`crates/librefang-wire/src/peer.rs`). A peer that
  does not know the secret cannot complete the handshake.
- **Per-message HMAC** on every post-handshake frame
  (`write_message_authenticated` / `read_message_authenticated`). An active
  attacker on the path cannot **forge** or **modify** a frame without
  detection.
- **Replay protection** via per-handshake nonces with a 5-minute time window
  and a hard cap on tracked nonces ([#3880](https://github.com/librefang/librefang/issues/3880)).
- **Recipient-bound HMAC** so a captured handshake cannot be replayed against
  a different federation peer that shares the same secret
  ([#3875](https://github.com/librefang/librefang/issues/3875)).
- **Per-peer message size cap** (`MAX_PEER_MESSAGE_BYTES = 64 KiB`) to prevent
  a federated peer from forcing the receiver's LLM to drain its budget on
  oversized prompts ([#3876](https://github.com/librefang/librefang/issues/3876)).

What it does **not** provide:

- **Confidentiality.** Frames are JSON over plain TCP. A passive on-path
  observer (corporate WiFi, ISP, cloud LB, k8s sidecar) reads every system
  prompt, user input, and LLM output exchanged between two kernels.

## Why we don't ship in-process TLS

We previously explored adding TLS 1.3 termination inside `librefang-wire`
(see closed PR #4001). The cost/benefit didn't land:

1. **The threat is rarely instantiated.** OFP is overwhelmingly used either
   on a single host or inside a trusted private network. Cross-internet
   federation is, today, approximately nobody.
2. **Operators have a much cheaper option that does the same job.** Running
   OFP behind a private overlay (WireGuard, Tailscale, Nebula, an SSH
   tunnel, or an in-cluster mesh like Linkerd / Cilium) gives confidentiality,
   peer reachability control, and key rotation in one piece — none of it
   our code, none of it our maintenance burden.
3. **In-tree TLS is a long-tailed maintenance commitment.** Per-peer
   keypairs, pin distribution, rotation, ciphersuite policy, and migration
   from plaintext peers all become permanent surface area we have to keep
   safe across rustls bumps.

The HMAC framing already blocks the attacks people care most about
(forgery, tampering, replay, cross-peer replay). Confidentiality is what
remains, and confidentiality is what overlays solve well.

## Recommended deployment

If you federate kernels across an untrusted network, **do not run OFP
directly on the public internet.** Pick one of:

- **WireGuard / Tailscale.** Each kernel host joins the same overlay; OFP
  binds and dials peers via overlay IPs. Simplest path; recommended default.
- **SSH tunnel.** For ad-hoc bridging between two known hosts.
- **Service mesh mTLS** (Istio, Linkerd, Cilium). For Kubernetes deployments
  already running a mesh.

Inside a single trusted network (home LAN, single-cloud VPC, single
Kubernetes namespace), running OFP directly is fine — the HMAC framing
covers active-attacker concerns at that scope.

## When we'd revisit

We will reopen #3874 and reconsider in-tree TLS if any of the following
becomes true:

- A real deployment surfaces where overlays are not workable (e.g. a
  multi-tenant federation where operators don't trust each other enough to
  share an overlay).
- Compliance requirements (SOC 2 Type II at the wire layer, FedRAMP,
  similar) force on-the-wire encryption regardless of network topology.
- The HMAC-only model is shown insufficient against a plausible attacker
  profile we did not consider.

Until then: plaintext-on-the-wire + HMAC framing + overlay-for-confidentiality
is the supported deployment model.
