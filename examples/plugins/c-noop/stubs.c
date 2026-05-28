/*
 * stubs.c — minimal allocator stubs for the c-noop plugin.
 *
 * The generated bindings (plugin.c) reference free/realloc/abort at link time,
 * but the noop run() implementation never returns an error string, so these
 * paths are dead code. Provide stubs to satisfy the linker without pulling in
 * a full WASI allocator.
 */
#include <stddef.h>

void  free(void *ptr)               { (void)ptr; }
void *malloc(size_t sz)             { (void)sz; return (void *)0; }
void *realloc(void *ptr, size_t sz) { (void)ptr; (void)sz; return (void *)0; }
void  abort(void)                   { __builtin_trap(); }
