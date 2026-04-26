import { describe, expect, it } from "vitest";
import { parseCsvText, parseUsersCsv } from "./csvParser";

// Mirrors UsersPage.tsx ROLES.
const ROLES = ["owner", "admin", "user", "viewer"] as const;

describe("parseCsvText", () => {
  it("handles simple comma-separated rows", () => {
    const out = parseCsvText("a,b,c\n1,2,3\n");
    expect(out.records).toEqual([
      ["a", "b", "c"],
      ["1", "2", "3"],
    ]);
  });

  it("strips a leading UTF-8 BOM", () => {
    // The BOM ("﻿") used to bleed into the first header cell.
    const out = parseCsvText("﻿name,role\nalice,admin\n");
    expect(out.records[0]).toEqual(["name", "role"]);
    // Sanity: only ONE BOM is stripped — a stray one mid-stream stays.
    const out2 = parseCsvText("name,role\n﻿alice,admin\n");
    expect(out2.records[1][0]).toBe("﻿alice");
  });

  it("preserves embedded newlines inside quoted fields", () => {
    // RFC-4180 §2.6: line breaks inside quotes are part of the field.
    const csv = 'name,note\n"alice","line1\nline2"\n"bob","x"\n';
    const out = parseCsvText(csv);
    expect(out.records).toEqual([
      ["name", "note"],
      ["alice", "line1\nline2"],
      ["bob", "x"],
    ]);
  });

  it("preserves embedded commas inside quoted fields", () => {
    const out = parseCsvText('a,b\n"hello, world","x"\n');
    expect(out.records[1]).toEqual(["hello, world", "x"]);
  });

  it("decodes escaped double-quotes (\"\")", () => {
    const out = parseCsvText('a\n"he said ""hi"""\n');
    expect(out.records[1]).toEqual(['he said "hi"']);
  });

  it("handles CRLF and CR-only line endings", () => {
    const crlf = parseCsvText("a,b\r\n1,2\r\n");
    expect(crlf.records).toEqual([
      ["a", "b"],
      ["1", "2"],
    ]);
    const cr = parseCsvText("a,b\r1,2\r");
    expect(cr.records).toEqual([
      ["a", "b"],
      ["1", "2"],
    ]);
  });

  it("does not emit a phantom record for trailing newline", () => {
    const out = parseCsvText("a,b\n1,2\n");
    expect(out.records).toHaveLength(2);
  });

  it("emits the final record when no trailing newline", () => {
    const out = parseCsvText("a,b\n1,2");
    expect(out.records).toEqual([
      ["a", "b"],
      ["1", "2"],
    ]);
  });
});

describe("parseUsersCsv", () => {
  it("parses a basic file", () => {
    const csv = "name,role,telegram\nalice,admin,123\nbob,user,456\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.errors).toEqual([]);
    expect(out.rows).toEqual([
      { name: "alice", role: "admin", channel_bindings: { telegram: "123" } },
      { name: "bob", role: "user", channel_bindings: { telegram: "456" } },
    ]);
  });

  it("imports a BOM-prefixed file (regression: import used to fail)", () => {
    const csv = "﻿name,role,telegram\nalice,admin,123\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.errors).toEqual([]);
    expect(out.rows).toHaveLength(1);
    expect(out.rows[0]).toEqual({
      name: "alice",
      role: "admin",
      channel_bindings: { telegram: "123" },
    });
  });

  it("imports rows with quoted-newline fields (regression)", () => {
    // The old parser split on \n first, so the second row was split into
    // ["bob", "\"with"] and ["embedded\""], both invalid.
    const csv =
      'name,role,note\n"alice","admin","line1\nline2"\n"bob","user","x"\n';
    const out = parseUsersCsv(csv, ROLES);
    expect(out.errors).toEqual([]);
    expect(out.rows).toHaveLength(2);
    expect(out.rows[0].channel_bindings.note).toBe("line1\nline2");
  });

  it("rejects a file missing the name column", () => {
    const csv = "role,telegram\nadmin,123\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows).toEqual([]);
    expect(out.errors[0]).toMatch(/name/);
  });

  it("rejects a file missing the role column", () => {
    const csv = "name,telegram\nalice,123\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows).toEqual([]);
    expect(out.errors[0]).toMatch(/role/);
  });

  it("flags invalid roles but still surfaces the row in preview", () => {
    const csv = "name,role\nalice,wizard\n";
    const out = parseUsersCsv(csv, ROLES);
    // Row is kept (preview), but error reports it.
    expect(out.rows).toHaveLength(1);
    expect(out.errors[0]).toMatch(/invalid role 'wizard'/);
  });

  it("skips rows with missing name", () => {
    const csv = "name,role\n,admin\nalice,user\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows).toHaveLength(1);
    expect(out.rows[0].name).toBe("alice");
    expect(out.errors[0]).toMatch(/Row 2: missing name/);
  });

  it("ignores blank lines between records", () => {
    const csv = "name,role\nalice,admin\n\n\nbob,user\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows.map(r => r.name)).toEqual(["alice", "bob"]);
  });

  it("treats unknown columns as channel bindings", () => {
    const csv = "name,role,telegram,discord\nalice,admin,123,abc#0001\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows[0].channel_bindings).toEqual({
      telegram: "123",
      discord: "abc#0001",
    });
  });

  it("drops empty channel-binding cells", () => {
    const csv = "name,role,telegram,discord\nalice,admin,,abc#0001\n";
    const out = parseUsersCsv(csv, ROLES);
    expect(out.rows[0].channel_bindings).toEqual({ discord: "abc#0001" });
  });
});
