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
//! ## Wire confidentiality
//!
//! OFP frames are **plaintext** on the wire. Authentication, integrity, and
//! replay protection are provided by an HMAC-SHA256 handshake plus per-message
//! HMAC; confidentiality is **not** provided here and must come from the
//! deployment (WireGuard / Tailscale / SSH tunnel / service-mesh mTLS).
//!
//! Do not add TLS termination inside this crate without first re-evaluating
//! the decision documented at <https://docs.librefang.ai/architecture/ofp-wire>
//! (closed issue #3874, closed PR #4001). The HMAC framing intentionally
//! covers active-attacker threats; overlays cover passive-observer threats.
//! Re-implementing TLS on top of that adds maintenance burden without
//! changing the supported deployment model.

pub mod message;
pub mod peer;
pub mod registry;

pub use message::{WireMessage, WireRequest, WireResponse};
pub use peer::{PeerConfig, PeerNode};
pub use registry::{PeerEntry, PeerRegistry, RemoteAgent};
