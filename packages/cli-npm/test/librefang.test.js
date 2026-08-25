"use strict";

const assert = require("node:assert/strict");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { childExitCode, resolveBinary } = require("../bin/librefang.js");

test("child exit status and signal deaths map to shell exit codes", () => {
  assert.equal(childExitCode({ status: 23 }), 23);
  assert.equal(
    childExitCode({ status: null, signal: "SIGTERM" }),
    128 + os.constants.signals.SIGTERM,
  );
  assert.equal(childExitCode({ status: null, signal: "UNKNOWN" }), 1);
});

test("binary resolution ignores only missing optional packages", () => {
  const missing = new Error("missing package");
  missing.code = "MODULE_NOT_FOUND";
  assert.equal(
    resolveBinary("@librefang/cli-test", "librefang", () => {
      throw missing;
    }),
    null,
  );

  const denied = new Error("permission denied");
  denied.code = "EACCES";
  assert.throws(
    () => resolveBinary("@librefang/cli-test", "librefang", () => {
      throw denied;
    }),
    denied,
  );
});

test("binary resolution checks the package bin directory", () => {
  const packageJson = path.join("tmp", "package", "package.json");
  const expected = path.join("tmp", "package", "bin", "librefang");
  assert.equal(
    resolveBinary("@librefang/cli-test", "librefang", () => packageJson, (candidate) => {
      assert.equal(candidate, expected);
      return true;
    }),
    expected,
  );
});
