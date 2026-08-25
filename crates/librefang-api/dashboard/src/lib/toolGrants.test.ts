import { describe, it, expect } from "vitest";
import {
  isMcpServerGranted,
  isToolAllowed,
  isToolBlocked,
  normalizeMcpName,
  resolveMcpGrantMode,
  toolPatternMatches,
} from "./toolGrants";

describe("toolPatternMatches", () => {
  it("matches everything for a bare star", () => {
    expect(toolPatternMatches("*", "anything")).toBe(true);
  });

  it("matches an exact literal", () => {
    expect(toolPatternMatches("file_read", "file_read")).toBe(true);
    expect(toolPatternMatches("file_read", "file_write")).toBe(false);
  });

  it("treats a wildcard-free pattern as exact-only", () => {
    expect(toolPatternMatches("file", "file_read")).toBe(false);
  });

  it("matches a prefix wildcard", () => {
    expect(toolPatternMatches("file_*", "file_read")).toBe(true);
    expect(toolPatternMatches("file_*", "shell_exec")).toBe(false);
  });

  it("matches a suffix wildcard", () => {
    expect(toolPatternMatches("*_read", "file_read")).toBe(true);
    expect(toolPatternMatches("*_read", "file_write")).toBe(false);
  });

  it("matches an interior wildcard", () => {
    expect(toolPatternMatches("mcp_*_search", "mcp_brave_search")).toBe(true);
    expect(toolPatternMatches("mcp_*_search", "mcp_brave_fetch")).toBe(false);
  });

  it("matches multi-wildcard patterns with the kernel's recursive prefix stripping", () => {
    expect(toolPatternMatches("a*b*c", "abc")).toBe(true);
    expect(toolPatternMatches("ab*cd*ef", "abcdef")).toBe(true);
    expect(toolPatternMatches("a*b*c", "axbxc")).toBe(false);
    expect(toolPatternMatches("ab*cd*ef", "abxcdyef")).toBe(false);
  });

  it("does not let prefix and suffix overlap the same characters", () => {
    // `a*b` requires at least "ab" worth of distinct characters.
    expect(toolPatternMatches("ab*ab", "abab")).toBe(true);
    expect(toolPatternMatches("ab*ab", "aba")).toBe(false);
  });

  it("matches a whole-server MCP glob", () => {
    expect(toolPatternMatches("mcp_brave_*", "mcp_brave_search")).toBe(true);
    expect(toolPatternMatches("mcp_brave_*", "mcp_github_search")).toBe(false);
  });
});

describe("isToolBlocked", () => {
  it("is false for an empty blocklist", () => {
    expect(isToolBlocked("mcp_brave_search", [])).toBe(false);
  });

  it("blocks an exact entry", () => {
    expect(isToolBlocked("mcp_brave_search", ["mcp_brave_search"])).toBe(true);
  });

  it("blocks via a glob entry", () => {
    expect(isToolBlocked("mcp_brave_search", ["mcp_brave_*"])).toBe(true);
    expect(isToolBlocked("mcp_github_search", ["mcp_brave_*"])).toBe(false);
  });
});

describe("normalizeMcpName", () => {
  it("lowercases and folds dashes to underscores", () => {
    expect(normalizeMcpName("Brave-Search")).toBe("brave_search");
  });
});

describe("resolveMcpGrantMode", () => {
  it("prefers the backend-supplied mode", () => {
    expect(resolveMcpGrantMode([], "all")).toBe("all");
  });

  it("derives none from an empty list", () => {
    expect(resolveMcpGrantMode([], undefined)).toBe("none");
    expect(resolveMcpGrantMode(undefined, undefined)).toBe("none");
  });

  it("derives all from a wildcard entry", () => {
    expect(resolveMcpGrantMode(["*"], undefined)).toBe("all");
  });

  it("derives allowlist from named servers", () => {
    expect(resolveMcpGrantMode(["brave"], undefined)).toBe("allowlist");
  });
});

describe("isMcpServerGranted", () => {
  it("grants nothing in none mode", () => {
    expect(isMcpServerGranted("brave", ["brave"], "none")).toBe(false);
  });

  it("grants every server in all mode", () => {
    expect(isMcpServerGranted("anything", ["*"], "all")).toBe(true);
  });

  it("grants only named servers in allowlist mode", () => {
    expect(isMcpServerGranted("brave", ["brave", "github"], "allowlist")).toBe(true);
    expect(isMcpServerGranted("linear", ["brave", "github"], "allowlist")).toBe(false);
  });

  it("compares server names after normalization", () => {
    expect(isMcpServerGranted("Brave-Search", ["brave_search"], "allowlist")).toBe(true);
  });
});

describe("isToolAllowed", () => {
  it("treats an empty allowlist as unrestricted", () => {
    expect(isToolAllowed("mcp__github__create_issue", [])).toBe(true);
    expect(isToolAllowed("file_read", [])).toBe(true);
  });

  // The kernel's Step 4 filter runs after MCP tools join the candidate set, so an allowlist naming only native tools strips a granted server entirely (#6495).
  it("drops mcp tools when the allowlist names only native tools", () => {
    expect(isToolAllowed("mcp__github__create_issue", ["file_read"])).toBe(false);
    expect(isToolAllowed("file_read", ["file_read"])).toBe(true);
  });

  it("keeps mcp tools matched by a glob entry", () => {
    expect(isToolAllowed("mcp__github__create_issue", ["mcp__github__*"])).toBe(true);
    expect(isToolAllowed("mcp__linear__list", ["mcp__github__*"])).toBe(false);
  });

  it("keeps everything under a bare star", () => {
    expect(isToolAllowed("mcp__github__create_issue", ["*"])).toBe(true);
  });
});
