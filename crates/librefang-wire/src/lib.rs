//! LibreFang Wire Protocol (OFP) — agent-to-agent networking.
//!
//! Provides cross-machine agent discovery, authentication, and communication
//! over TCP connections using a JSON-RPC framed protocol.
//!
//! ## Architecture
//!
//! - **PeerNode**: Local network endpoint that listens for incoming connections
//! - **PeerRegistry**: Tracks known peers and their agents
//! - **WireMessage**: JSON-framed protocol messages
//! - **PeerHandle**: Trait for routing remote messages through the kernel
//!
//! ## Authentication model
//!
//! Two layers, both required for a successful handshake:
//!
//! 1. **Network admission** — HMAC-SHA256 over `nonce | sender_node_id |
//!    recipient_node_id` using `shared_secret`. Coarse "do you have the
//!    cluster password" gate, bound to a specific `(sender, recipient)`
//!    pair so a captured packet cannot be replayed against a different
//!    mesh node (#3875).
//! 2. **Per-peer identity** (#3873) — every node persists an Ed25519
//!    keypair in `<data_dir>/peer_keypair.json`. The handshake also
//!    carries the sender's pubkey plus an Ed25519 signature over the
//!    same auth-data string the HMAC covers. Recipients verify the
//!    signature and TOFU-pin the pubkey to the sender's `node_id`.
//!    Subsequent handshakes claiming the same `node_id` MUST present
//!    the same pubkey or are rejected. Pins persist across restarts in
//!    `<data_dir>/trusted_peers.json`.
//!
//! Net effect: a leaked `shared_secret` no longer lets an attacker
//! impersonate a previously-pinned peer — they would also need that
//! node's private key. They can still open a connection under a fresh
//! `node_id` (admission gate is symmetric) but cannot pretend to be an
//! existing identity. Operators verify the local fingerprint via
//! `GET /api/network/status` (`identity_fingerprint`) and compare it
//! out-of-band with the peer they intend to federate with.
//!
//! ## Wire confidentiality
//!
//! OFP frames are **plaintext** on the wire. Authentication, integrity,
//! and replay protection are provided in this crate; confidentiality is
//! **not**, and must come from the deployment (WireGuard / Tailscale /
//! SSH tunnel / service-mesh mTLS).
//!
//! Do not add TLS termination inside this crate without first
//! re-evaluating the decision documented at
//! <https://docs.librefang.ai/architecture/ofp-wire> (closed issue
//! #3874, closed PR #4001). The HMAC + Ed25519 framing intentionally
//! covers active-attacker threats; overlays cover passive-observer
//! threats. Re-implementing TLS on top of that adds maintenance burden
//! without changing the supported deployment model.

pub mod keys;
pub mod message;
pub mod peer;
pub mod registry;
pub mod trusted_peers;

pub use message::{WireMessage, WireRequest, WireResponse};
pub use peer::{PeerConfig, PeerNode};
pub use registry::{PeerEntry, PeerRegistry, RemoteAgent};
