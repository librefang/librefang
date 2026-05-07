//! [`kernel_handle::WikiAccess`] — durable markdown knowledge vault
//! (issue #3329). The trait ships default impls that return
//! `KernelOpError::unavailable("wiki_*")`, so until the matching
//! `wiki_vault` field lands on `LibreFangKernel` (will arrive with the
//! #3329 main-side merge), this impl block is intentionally empty —
//! every method falls through to the trait default.

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::WikiAccess for LibreFangKernel {}
