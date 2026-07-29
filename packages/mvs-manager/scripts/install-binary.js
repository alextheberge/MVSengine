#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
"use strict";

/**
 * Download a pinned (or latest) mvs-manager release archive, verify SHA-256
 * against checksums.txt, and extract the binary next to the npm bin shim.
 */
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { pipeline } = require("node:stream/promises");
const { createWriteStream } = require("node:fs");

const REPO = process.env.MVS_REPO || "alextheberge/MVSengine";
const VERSION = process.env.MVS_VERSION || require("../package.json").version;
const BIN_DIR = path.join(__dirname, "..", "bin");

function detectTarget() {
  const platform = os.platform();
  const arch = os.arch();
  if (platform === "darwin") {
    if (arch === "arm64") return "aarch64-apple-darwin";
    if (arch === "x64") return "x86_64-apple-darwin";
  }
  if (platform === "linux" && arch === "x64") {
    return "x86_64-unknown-linux-gnu";
  }
  if (platform === "win32" && arch === "x64") {
    return "x86_64-pc-windows-msvc";
  }
  throw new Error(
    `Unsupported platform ${platform}/${arch}. Install via scripts/install.sh or build from source.`
  );
}

function tagFromVersion(version) {
  const v = String(version).replace(/^v/, "");
  return `v${v}`;
}

function fetchBuffer(url) {
  return new Promise((resolve, reject) => {
    const request = (currentUrl, redirects = 0) => {
      https
        .get(currentUrl, { headers: { "User-Agent": "mvs-manager-npm" } }, (res) => {
          if (
            res.statusCode >= 300 &&
            res.statusCode < 400 &&
            res.headers.location &&
            redirects < 5
          ) {
            res.resume();
            request(res.headers.location, redirects + 1);
            return;
          }
          if (res.statusCode !== 200) {
            reject(new Error(`GET ${currentUrl} failed: HTTP ${res.statusCode}`));
            res.resume();
            return;
          }
          const chunks = [];
          res.on("data", (c) => chunks.push(c));
          res.on("end", () => resolve(Buffer.concat(chunks)));
        })
        .on("error", reject);
    };
    request(url);
  });
}

async function downloadToFile(url, dest) {
  await new Promise((resolve, reject) => {
    const request = (currentUrl, redirects = 0) => {
      https
        .get(currentUrl, { headers: { "User-Agent": "mvs-manager-npm" } }, (res) => {
          if (
            res.statusCode >= 300 &&
            res.statusCode < 400 &&
            res.headers.location &&
            redirects < 5
          ) {
            res.resume();
            request(res.headers.location, redirects + 1);
            return;
          }
          if (res.statusCode !== 200) {
            reject(new Error(`GET ${currentUrl} failed: HTTP ${res.statusCode}`));
            res.resume();
            return;
          }
          const out = createWriteStream(dest);
          pipeline(res, out).then(resolve).catch(reject);
        })
        .on("error", reject);
    };
    request(url);
  });
}

function sha256File(filePath) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

function expectedSha(checksumsText, archiveName) {
  for (const line of checksumsText.split(/\r?\n/)) {
    const match = line.trim().match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (!match) continue;
    if (path.basename(match[2]) === archiveName) {
      return match[1].toLowerCase();
    }
  }
  return null;
}

async function main() {
  if (process.env.MVS_SKIP_BINARY_DOWNLOAD === "1") {
    console.log("MVS_SKIP_BINARY_DOWNLOAD=1; skipping binary install.");
    return;
  }

  const target = detectTarget();
  const tag = tagFromVersion(VERSION);
  const versionLabel = tag.replace(/^v/, "");
  const isWindows = target.includes("windows");
  const archiveExt = isWindows ? "zip" : "tar.gz";
  const archiveName = `mvs-manager-${versionLabel}-${target}.${archiveExt}`;
  const base = `https://github.com/${REPO}/releases/download/${tag}`;
  const archiveUrl = `${base}/${archiveName}`;
  const checksumsUrl = `${base}/checksums.txt`;

  fs.mkdirSync(BIN_DIR, { recursive: true });
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "mvs-manager-"));
  const archivePath = path.join(tmpDir, archiveName);

  try {
    console.log(`Downloading ${archiveUrl}`);
    await downloadToFile(archiveUrl, archivePath);
    const checksums = (await fetchBuffer(checksumsUrl)).toString("utf8");
    const expected = expectedSha(checksums, archiveName);
    if (!expected) {
      throw new Error(`No checksum line for ${archiveName} in checksums.txt`);
    }
    const actual = sha256File(archivePath);
    if (actual !== expected) {
      throw new Error(`Checksum mismatch for ${archiveName}: expected ${expected}, got ${actual}`);
    }

    if (isWindows) {
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-Command",
          `Expand-Archive -LiteralPath '${archivePath.replace(/'/g, "''")}' -DestinationPath '${tmpDir.replace(/'/g, "''")}' -Force`,
        ],
        { stdio: "inherit" }
      );
      const exe = path.join(tmpDir, "mvs-manager.exe");
      if (!fs.existsSync(exe)) {
        // search recursively
        const found = findFile(tmpDir, "mvs-manager.exe");
        if (!found) throw new Error("mvs-manager.exe not found in archive");
        fs.copyFileSync(found, path.join(BIN_DIR, "mvs-manager.exe"));
      } else {
        fs.copyFileSync(exe, path.join(BIN_DIR, "mvs-manager.exe"));
      }
    } else {
      execFileSync("tar", ["-xzf", archivePath, "-C", tmpDir], { stdio: "inherit" });
      const found =
        findFile(tmpDir, "mvs-manager") ||
        (fs.existsSync(path.join(tmpDir, "mvs-manager"))
          ? path.join(tmpDir, "mvs-manager")
          : null);
      if (!found) throw new Error("mvs-manager binary not found in archive");
      const dest = path.join(BIN_DIR, "mvs-manager-bin");
      fs.copyFileSync(found, dest);
      fs.chmodSync(dest, 0o755);
    }

    console.log(`Installed mvs-manager ${tag} (${target}) into ${BIN_DIR}`);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

function findFile(root, name) {
  const entries = fs.readdirSync(root, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(root, entry.name);
    if (entry.isFile() && entry.name === name) return full;
    if (entry.isDirectory()) {
      const nested = findFile(full, name);
      if (nested) return nested;
    }
  }
  return null;
}

main().catch((error) => {
  console.error(`mvs-manager postinstall failed: ${error.message}`);
  console.error(
    "You can still install via: curl -fsSL https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.sh | bash"
  );
  process.exit(1);
});
