//! Daemon-attached Agent Client Protocol (ACP) listener — Windows
//! named-pipe variant of [`crate::acp_uds`] (#3313).
//!
//! Wire-compatible with the Unix UDS path: same JSON-RPC framing,
//! same per-connection `KernelAdapter`, same shared kernel. Only the
//! transport layer differs — Unix uses
//! `tokio::net::UnixListener`/`UnixStream`, Windows uses
//! `tokio::net::windows::named_pipe::NamedPipeServer`.
//!
//! Pipe naming: `\\.\pipe\librefang-acp` (host-local). The CLI side
//! (`librefang-cli::acp::run_pipe_proxy`) connects via
//! `NamedPipeClient` and pipes stdin↔pipe↔stdout.
//!
//! # Trust model
//!
//! Same as the Unix UDS path: same-user, same-host. The default DACL
//! Windows applies to a named pipe grants `GENERIC_READ`/
//! `GENERIC_WRITE` to anyone on the local machine, which on a
//! Terminal Server / shared workstation means another logged-in user
//! could connect and drive the daemon's kernel. We harden by
//! installing an explicit DACL via SDDL `D:P(A;;GA;;;OW)` — only the
//! **OWNER** (the daemon's own user SID) gets `GENERIC_ALL`. The
//! `P` flag blocks DACL inheritance from the parent so a permissive
//! ACL on `\\.\pipe\` can't widen us back. Every pipe instance — the
//! initial bind and every rebind after a connect — is created with
//! this descriptor; the original code only set
//! `first_pipe_instance(true)` on the very first instance, leaving
//! the rebind path open to a name-squatting race.

#![cfg(windows)]

use std::sync::Arc;

use librefang_acp::{AcpKernel, KernelAdapter};
use librefang_kernel::LibreFangKernel;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{info, warn};

const DEFAULT_AGENT: &str = "assistant";

/// Canonical pipe name. Local-only namespace (`\\.\pipe\…`). The CLI
/// side hard-codes the same name when reaching for daemon-attached
/// mode — keep them in sync.
pub const PIPE_NAME: &str = r"\\.\pipe\librefang-acp";

/// Accept connections on the librefang ACP named pipe and serve ACP
/// over each. Returns only when the bg-task harness aborts the
/// listener loop.
pub async fn run_listener(kernel: Arc<LibreFangKernel>) -> std::io::Result<()> {
    info!(pipe = %PIPE_NAME, "ACP named-pipe listener bound (DACL: owner-only)");
    // Named pipes work differently from UDS — there's no single
    // listener accepting multiple `connect()` calls. Instead, the
    // server creates one pipe instance, awaits a connect, then
    // creates the next instance for the next connection. The
    // `first_pipe_instance(true)` on the very first server prevents
    // a second daemon from racing in on the same name.
    let mut server = create_owner_only_instance(true)?;

    loop {
        if let Err(e) = server.connect().await {
            warn!(error = %e, "ACP named-pipe connect failed");
            // Drop the busted instance and rebind for the next
            // attempt. Returning here would tear the listener down
            // for the rest of the daemon's lifetime.
            server = create_owner_only_instance(false)?;
            continue;
        }
        // Hand the connected pipe off to a background task and
        // immediately create the next instance so the next CLI
        // invocation isn't blocked behind this connection's
        // lifetime.
        let connected = server;
        server = create_owner_only_instance(false)?;
        let kernel = Arc::clone(&kernel);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(kernel, connected).await {
                warn!(error = %e, "ACP named-pipe connection ended with error");
            }
        });
    }
}

/// Build a `NamedPipeServer` instance with an owner-only DACL.
///
/// `first` determines whether `first_pipe_instance(true)` is set —
/// only the very first call after process start should pass `true`,
/// or a stale instance from a crashed daemon could be hijacked by a
/// local attacker who races us to `CreateNamedPipe` between the old
/// daemon dying and us coming back up. Subsequent rebinds (every
/// time we hand a connected pipe off to a worker and need a fresh
/// listener) pass `false` because by then `first_pipe_instance` would
/// reject us.
fn create_owner_only_instance(first: bool) -> std::io::Result<NamedPipeServer> {
    let descriptor = OwnerOnlyDescriptor::new()?;
    let mut sa = SecurityAttributes::new(&descriptor);
    let mut options = ServerOptions::new();
    if first {
        options.first_pipe_instance(true);
    }
    // Reject anything not from the same machine. Default-on for
    // tokio's named-pipe builder, restate explicitly for clarity.
    options.reject_remote_clients(true);
    // SAFETY: `sa.as_raw()` returns a pointer that stays valid for
    // the lifetime of `sa`, which lives until end of this function;
    // `create_with_security_attributes_raw` consumes the pointer
    // before returning, so the dangling-pointer lifetime constraint
    // is met.
    unsafe { options.create_with_security_attributes_raw(PIPE_NAME, sa.as_raw_mut()) }
}

async fn handle_connection(
    kernel: Arc<LibreFangKernel>,
    stream: NamedPipeServer,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = Arc::new(KernelAdapter::new(kernel));
    let agent_id = adapter.resolve_agent(DEFAULT_AGENT).await?;

    // `tokio::io::split` for `NamedPipeServer` returns `ReadHalf` /
    // `WriteHalf` that implement `AsyncRead` / `AsyncWrite`. The
    // ACP transport wants `futures::AsyncRead` / `AsyncWrite`, so we
    // adapt through `tokio_util::compat`.
    let (read_half, write_half) = tokio::io::split(stream);
    let transport =
        agent_client_protocol::ByteStreams::new(write_half.compat_write(), read_half.compat());
    librefang_acp::run_with_transport(adapter, agent_id, transport).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Owner-only security descriptor (Windows DACL).
// ---------------------------------------------------------------------------
//
// We build the descriptor from an SDDL string — `D:P(A;;GA;;;OW)`
// translates as: protected DACL, with one ACE granting GENERIC_ALL
// to OWNER. `OW` means "owner of the object" — the named pipe is
// owned by the process creating it, so this resolves to the
// daemon's user SID at runtime. `P` blocks inheritance.
//
// This is intentionally minimal: a single owner-allow ACE, no
// inherited permissions, no group/user widening. If a future
// requirement adds explicit administrator override, a second ACE
// can be appended (`(A;;GA;;;BA)` for BUILTIN\Administrators) — but
// the principle of least privilege says start tight and widen only
// for documented use cases.

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

/// `D:P(A;;GA;;;OW)` — protected DACL, GENERIC_ALL to owner only.
///
/// Constants below are the UTF-16 representation we hand to
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW`. Encoded
/// inline rather than calling `OsStr::encode_wide` so the slice is
/// `&'static [u16]` and we don't allocate per call.
const SDDL: &[u16] = &[
    'D' as u16, ':' as u16, 'P' as u16, '(' as u16, 'A' as u16, ';' as u16, ';' as u16, 'G' as u16,
    'A' as u16, ';' as u16, ';' as u16, ';' as u16, 'O' as u16, 'W' as u16, ')' as u16, 0,
];

struct OwnerOnlyDescriptor {
    raw: PSECURITY_DESCRIPTOR,
}

impl OwnerOnlyDescriptor {
    fn new() -> std::io::Result<Self> {
        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: SDDL is a NUL-terminated UTF-16 buffer with static
        // lifetime; the function writes the resulting descriptor
        // pointer into `psd` on success and we own the cleanup via
        // `LocalFree` in `Drop`.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                SDDL.as_ptr(),
                SDDL_REVISION_1,
                &mut psd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || psd.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { raw: psd })
    }
}

impl Drop for OwnerOnlyDescriptor {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `raw` came from
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW`
            // (or is null, which `LocalFree` accepts as a no-op).
            unsafe {
                LocalFree(self.raw as _);
            }
        }
    }
}

/// `SECURITY_ATTRIBUTES` wrapping a borrowed descriptor.
struct SecurityAttributes<'a> {
    inner: SECURITY_ATTRIBUTES,
    _marker: std::marker::PhantomData<&'a OwnerOnlyDescriptor>,
}

impl<'a> SecurityAttributes<'a> {
    fn new(desc: &'a OwnerOnlyDescriptor) -> Self {
        Self {
            inner: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: desc.raw,
                bInheritHandle: 0,
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Pointer suitable for passing as `*mut c_void`. Caller must not
    /// retain it past `self`'s lifetime.
    fn as_raw_mut(&mut self) -> *mut std::ffi::c_void {
        &mut self.inner as *mut _ as *mut std::ffi::c_void
    }
}
