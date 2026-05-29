//go:build tinygo || wasm

package main

import "unsafe"

// cabi_realloc is the Component Model canonical ABI realloc function.
// wasm-tools component new requires this export for any component that
// passes heap-allocated types (strings, slices) across the host↔guest
// boundary.
//
// Implementation: delegates to TinyGo's leaking GC heap via make([]byte, n).
//
// WHY make() IS SAFE HERE (with the wasmtime v45 reactor adapter):
//   The WASI P1 reactor adapter (wasi-preview1-component-adapter-provider@45.0.0)
//   initialises its internal State LAZILY on first use. The first use happens
//   during _initialize → initRand() → arc4random → random_get → State::with().
//   At that point, initHeap() has ALREADY run (it is the first call inside
//   wasmEntryReactor), so the Go heap is available for make().
//
// CONTRAST WITH BROKEN APPROACH:
//   The Phase-7 adapter (wit-bindgen-cli-0.57.1, 94 KB) initialised its State
//   EAGERLY during module instantiation — before _initialize, before initHeap().
//   Using make() there caused "alloc called before initHeap" panics.
//   The v45 adapter (52 KB) does NOT do eager initialisation, so make() is safe.
//
// NOTE: This implementation leaks allocations (never frees). The leaking GC
// (-gc=leaking) is used for the whole program, so this is consistent.

//export cabi_realloc
func cabi_realloc(ptr unsafe.Pointer, origSize, align, newSize uintptr) unsafe.Pointer {
	if newSize == 0 {
		// Canonical "free" — leaking allocator, just return nil.
		return nil
	}
	buf := make([]byte, newSize)
	if ptr != nil && origSize > 0 {
		copyLen := origSize
		if copyLen > newSize {
			copyLen = newSize
		}
		copy(buf, (*[1 << 28]byte)(ptr)[:copyLen])
	}
	return unsafe.Pointer(unsafe.SliceData(buf))
}
