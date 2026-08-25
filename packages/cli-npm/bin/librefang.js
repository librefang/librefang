#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const PLATFORM_MAP = {
  linux:  "linux",
  darwin: "darwin",
  win32:  "win32",
};

const ARCH_MAP = {
  x64:   "x64",
  arm64: "arm64",
};

function resolveBinary(pkg, exe, resolver = require.resolve, exists = fs.existsSync) {
  try {
    const pkgDir = path.dirname(resolver(`${pkg}/package.json`));
    const bin = path.join(pkgDir, "bin", exe);
    return exists(bin) ? bin : null;
  } catch (err) {
    if (err && err.code === "MODULE_NOT_FOUND") return null;
    throw err;
  }
}

function getBinaryPath() {
  const platform = PLATFORM_MAP[process.platform];
  const arch = ARCH_MAP[process.arch];

  if (!platform || !arch) {
    console.error(`Unsupported platform: ${process.platform} ${process.arch}`);
    process.exit(1);
  }

  // Try glibc variant first, then musl for Linux
  const candidates = [`@librefang/cli-${platform}-${arch}`];
  if (platform === "linux") {
    candidates.push(`@librefang/cli-${platform}-${arch}-musl`);
  }

  const exe = process.platform === "win32" ? "librefang.exe" : "librefang";

  for (const pkg of candidates) {
    const bin = resolveBinary(pkg, exe);
    if (bin) return bin;
  }

  console.error(
    `Could not find librefang binary for ${process.platform}-${process.arch}.\n` +
    `Try: npm install @librefang/cli-${platform}-${arch}`
  );
  process.exit(1);
}

function childExitCode(err) {
  if (Number.isInteger(err && err.status)) return err.status;
  const signalNumber = err && err.signal && os.constants.signals[err.signal];
  return Number.isInteger(signalNumber) ? 128 + signalNumber : 1;
}

function main() {
  const binary = getBinaryPath();
  try {
    execFileSync(binary, process.argv.slice(2), { stdio: "inherit" });
  } catch (err) {
    process.exit(childExitCode(err));
  }
}

if (require.main === module) {
  main();
}

module.exports = { childExitCode, resolveBinary };
