Link understanding now compiles its URL extraction pattern once and shares it across messages, avoiding repeated regex parsing and allocation on the message processing path. (#6912) (@houko)
