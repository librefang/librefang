`PUT /api/templates/{name}` no longer panics the daemon on a non-object JSON body.
The handler took a `Json<serde_json::Value>`, which happily deserializes an array, string, number or bool, and then wrote `body["name"] = ...` to pin the manifest name to the URL path segment — but `Value`'s `IndexMut<&str>` only handles `Null` and `Object`, so every other variant panicked.
Any caller could trip it on an existing agent type with a one-line body of `[]` or `42`, no authentication bypass required.
The route now takes a typed `AgentTypeSpec`, so a body that is not an object is rejected by deserialization before any handler code runs, and the name is pinned by assigning the path segment to the manifest rather than by indexing into untrusted JSON (#7859) (@DaBlitzStein)
