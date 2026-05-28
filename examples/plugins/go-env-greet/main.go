// go-env-greet — Phase-6 plugin example exercising the `env`
// host capability through the librefang:plugin world.
//
// Reads the GREETING_NAME env var via env.Read and returns Ok.
// If the var is not set, the plugin still succeeds — the env
// capability gate (Phase-5 C-005) is exercised regardless of
// whether the var exists.
//
// Build:  cargo xtask plugins-rebuild go-env-greet
package main

import (
	"go.bytecodealliance.org/cm"
envimport "librefang.io/plugin/go-env-greet/librefang/plugin/env"
	plugin "librefang.io/plugin/go-env-greet/librefang/plugin/plugin"
	plugintypes "librefang.io/plugin/go-env-greet/librefang/plugin/plugin-types"
)

func init() {
	plugin.Exports.Run = run
}

func run() cm.Result[plugintypes.PluginError, struct{}, plugintypes.PluginError] {
	var out cm.Result[plugintypes.PluginError, struct{}, plugintypes.PluginError]
	// Read GREETING_NAME — capability-checked by the host, returns
	// none if the var is absent and capability-denied if not allowed.
	result := envimport.Read("GREETING_NAME")
	if result.IsErr() {
		err := result.Err()
		msg := "env.read failed"
		if err != nil {
			msg = err.String()
		}
		out.SetErr(plugintypes.PluginErrorInternal(msg))
		return out
	}
	// Success — whether the var is set or not.
	out.SetOK(struct{}{})
	return out
}

func main() {}
