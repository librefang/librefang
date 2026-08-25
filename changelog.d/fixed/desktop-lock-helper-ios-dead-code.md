The iOS and Android builds compile again after `lock_server_handle` was left without the desktop-only `cfg` its only caller carries.
The attribute above it applied to the `ServerHandleHolder` struct, not to the function that follows it, so mobile targets compiled a helper nothing there calls and `-D warnings` turned the resulting dead-code lint into a build failure.
Every desktop CI lane stayed green because none of them cross-compiles to an iOS target, which is why this reached `main`.
(#7934) (@houko)
