#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import process from "node:process";

import { ensureInstalledBinary } from "../lib/install.mjs";

const binaryPath = await ensureInstalledBinary({
  allowDownload: true
});

const child = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: "inherit"
});

if (child.error) {
  throw child.error;
}

process.exit(child.status ?? 1);
