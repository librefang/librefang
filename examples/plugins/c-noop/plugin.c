/*
 * c-noop — Phase-6 plugin example with no host capabilities.
 *
 * Implements exports_plugin_run() from the wit-bindgen generated
 * bindings/plugin.h and immediately returns Ok(()). No host imports
 * are used; the link-time gate enforces an empty host_capabilities list.
 *
 * Build:  cargo xtask plugins-rebuild c-noop
 */
#include "bindings/plugin.h"

/*
 * The single entry point required by the plugin world.
 * Returns true = Ok(()); returns false + fills *err = Err(plugin_error).
 * We always succeed.
 */
bool exports_plugin_run(plugin_plugin_error_t *err)
{
    (void)err;   /* unused — we always return Ok */
    return true; /* true = Ok(()) */
}
