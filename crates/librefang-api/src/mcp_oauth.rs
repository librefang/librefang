//! MCP OAuth provider re-export — keeps API call sites off
//! `librefang_kernel::mcp_oauth_provider` so the kernel internal module path
//! is not part of the API crate's public dependencies.
//!
//! Part of the issue #3744 effort to narrow API → kernel internal imports.
//! New code in `librefang-api` should reach for [`KernelOAuthProvider`] via
//! this re-export rather than poking at `librefang_kernel::mcp_oauth_provider`
//! directly. The underlying type still lives in the kernel because it is
//! constructed by the kernel for its own MCP runtime wiring (see
//! `librefang_kernel::kernel::Kernel::mcp_oauth_provider`).

pub use librefang_kernel::mcp_oauth_provider::KernelOAuthProvider;
