The MCP server form now has manual OAuth fields (authorization URL, token URL, client ID, client-secret env var name and scopes) for servers whose OAuth metadata cannot be discovered, so such a server no longer needs a hand-edited `[mcp_servers.oauth]` block in `config.toml`.
  The MCP list and detail routes now return the `oauth` block, and `PUT /api/mcp/servers/{name}` treats an absent `oauth` key as unchanged, so editing any other field no longer silently deletes a configured block.
  An explicit `"oauth": null` clears it. (#8670) (@GQAdonis)
