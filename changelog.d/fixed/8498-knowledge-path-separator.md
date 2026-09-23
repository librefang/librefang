On Windows, the knowledge base listing and create response reported a base's path as `knowledge\<name>`, and sharing a base wrote that backslash form into `agent.toml`.
  The path is now always `knowledge/<name>`, the form the documentation and every other host use, and existing declarations written with either separator keep matching (#8498) (@houko)
