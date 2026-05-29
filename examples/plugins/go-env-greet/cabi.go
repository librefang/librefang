//go:build tinygo || wasm

package main

import "unsafe"

// cabi_realloc is the Component Model canonical ABI realloc function.
// wasm-tools component new requires this export for any component that
// passes heap-allocated types (strings, slices) across the host↔guest
// boundary.
//
// Implementation: static bump allocator over a pre-allocated 256 KB arena.
//
// WHY NOT make() / runtime.alloc:
//   The WASI P1 reactor adapter calls cabi_realloc during component
//   instantiation — BEFORE the core module's _initialize has run —
//   to allocate its own internal state. But _initialize is what calls
//   initHeap(), so make() / runtime.alloc would panic ("alloc called
//   before initHeap") if invoked at that point.
//
// WHY A STATIC ARENA:
//   WASM globals are placed in the data segment, which is initialised as
//   part of module instantiation (before any function is called). The
//   arena and arenaPos variables are therefore accessible before
//   _initialize runs, and no dynamic allocation is needed.
//
// 256 KB headroom: the WASI P1 adapter state + any librefang string
//   parameters fit comfortably. The go-env-greet plugin never returns a
//   string in an error path (it always returns Ok), so heap demand is low.

const arenaSize = 256 * 1024 // 256 KB

var cabiArena [arenaSize]byte
var arenaPos uintptr

//export cabi_realloc
func cabi_realloc(ptr unsafe.Pointer, origSize, align, newSize uintptr) unsafe.Pointer {
	if newSize == 0 {
		return nil
	}

	// Align the bump pointer.
	if align > 0 {
		arenaPos = (arenaPos + align - 1) &^ (align - 1)
	}

	if arenaPos+newSize > arenaSize {
		// Bump allocator exhausted — shouldn't happen for a minimal plugin.
		// Trap rather than returning a null/invalid pointer.
		panic("cabi_realloc: arena exhausted (increase arenaSize in cabi.go)")
	}

	result := unsafe.Pointer(&cabiArena[arenaPos])

	// Copy existing content when resizing.
	if ptr != nil && origSize > 0 {
		copyLen := origSize
		if copyLen > newSize {
			copyLen = newSize
		}
		copy(
			unsafe.Slice((*byte)(result), newSize),
			unsafe.Slice((*byte)(ptr), copyLen),
		)
	}

	arenaPos += newSize
	return result
}
