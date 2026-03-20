#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const fs = require("fs");
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
    try {
      const pkgDir = path.dirname(require.resolve(`${pkg}/package.json`));
      const bin = path.join(pkgDir, "bin", exe);
      if (fs.existsSync(bin)) return bin;
    } catch {}
  }

  console.error(
    `Could not find librefang binary for ${process.platform}-${process.arch}.\n` +
    `Try: npm install @librefang/cli-${platform}-${arch}`
  );
  process.exit(1);
}

try {
  execFileSync(getBinaryPath(), process.argv.slice(2), { stdio: "inherit" });
} catch (err) {
  process.exit(err.status ?? 1);
}
