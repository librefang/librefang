Saving an agent's MCP server allowlist as `["*"]` no longer fails with `Unknown MCP server '*'`.
  `"*"` is the allowlist's own vocabulary for "every connected MCP server", distinct from the empty list, but the save path validated it as if it were a server name and looked it up in config.toml and the installed catalog — which it is never in.
  Choosing "all servers" from the dashboard was therefore the one option the route could not accept.
  An unknown name alongside the wildcard is still rejected. (#5855) (@DaBlitzStein)
