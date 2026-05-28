//go:build tinygo || wasm

package main

import "unsafe"

// cabi_realloc is the Component Model canonical ABI realloc function.
// wasm-tools component new requires this export for components that pass
// heap-allocated types (strings, slices) across the host-guest boundary.
//
//export cabi_realloc
func cabi_realloc(ptr unsafe.Pointer, origSize, align, newSize uintptr) unsafe.Pointer {
	if newSize == 0 {
		return nil
	}
	newBuf := make([]byte, newSize)
	if origSize > 0 && ptr != nil {
		copy(newBuf, unsafe.Slice((*byte)(ptr), origSize))
	}
	return unsafe.Pointer(unsafe.SliceData(newBuf))
}
