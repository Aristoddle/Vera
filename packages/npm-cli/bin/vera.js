#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const http = require("node:http");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { version: packageVersion } = require("../package.json");

const DEFAULT_REPO = "VeraTools/Vera";
const MAX_REDIRECTS = 5;

function parseArgs(argv) {
  if (argv.length === 0) {
    return { command: "help", rest: [] };
  }

  return { command: argv[0], rest: argv.slice(1) };
}

function detectMusl() {
  if (process.platform !== "linux") return false;
  try {
    const result = spawnSync("ldd", ["--version"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    const output = (result.stdout || "").toString() + (result.stderr || "").toString();
    return /musl/i.test(output);
  } catch {
    // Fallback: check for musl dynamic linker.
    try {
      const entries = fs.readdirSync("/lib");
      return entries.some((e) => e.startsWith("ld-musl-"));
    } catch {
      return false;
    }
  }
}

function resolveTarget(platform, arch) {
  const override = process.env.VERA_TARGET;
  if (override) return override;

  const key = `${platform}:${arch}`;
  const targets = {
    "linux:x64": detectMusl()
      ? "x86_64-unknown-linux-musl"
      : "x86_64-unknown-linux-gnu",
    "linux:arm64": "aarch64-unknown-linux-gnu",
    "darwin:x64": "x86_64-apple-darwin",
    "darwin:arm64": "aarch64-apple-darwin",
    "win32:x64": "x86_64-pc-windows-msvc",
  };

  const target = targets[key];
  if (!target) {
    throw new Error(`unsupported platform: ${platform}/${arch}`);
  }

  return target;
}

function defaultReleaseBaseUrl() {
  return process.env.VERA_RELEASE_BASE_URL || `https://github.com/${DEFAULT_REPO}`;
}

function manifestUrl(version) {
  if (process.env.VERA_MANIFEST_URL) {
    return process.env.VERA_MANIFEST_URL;
  }

  return `${defaultReleaseBaseUrl()}/releases/download/v${version}/release-manifest.json`;
}

function latestManifestUrl() {
  return `${defaultReleaseBaseUrl()}/releases/latest/download/release-manifest.json`;
}

function defaultVeraHome() {
  return process.env.VERA_HOME || path.join(os.homedir(), ".vera");
}

function installMetadataPath() {
  return path.join(defaultVeraHome(), "install.json");
}

function currentInstallMethod() {
  if (process.versions.bun) return "bun";
  const ua = process.env.npm_config_user_agent || "";
  if (ua.startsWith("bun/")) return "bun";
  const execpath = process.env.npm_execpath || "";
  if (execpath.includes("bun")) return "bun";
  return "npm";
}

async function readInstallMetadata() {
  try {
    const raw = await fsp.readFile(installMetadataPath(), "utf8");
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

async function writeInstallMetadata({ installMethod, version, binaryPath, target }) {
  const metadataPath = installMetadataPath();
  await fsp.mkdir(path.dirname(metadataPath), { recursive: true });
  const current = await readInstallMetadata();
  const next = {
    install_method: installMethod ?? current.install_method ?? null,
    version: version ?? current.version ?? null,
    binary_path: binaryPath ?? current.binary_path ?? null,
    target: target ?? current.target ?? null,
  };
  const tmpPath = `${metadataPath}.tmp.${process.pid}`;
  await fsp.writeFile(tmpPath, `${JSON.stringify(next, null, 2)}\n`, "utf8");
  await fsp.rename(tmpPath, metadataPath);
}

function preferredBinDirs() {
  const home = os.homedir();
  if (process.env.VERA_USER_BIN_DIR) {
    return [process.env.VERA_USER_BIN_DIR];
  }

  if (process.platform === "win32") {
    return [
      path.join(home, "AppData", "Roaming", "npm"),
      path.join(home, "AppData", "Local", "Programs", "Vera", "bin"),
    ];
  }

  return [
    path.join(home, ".local", "bin"),
    path.join(home, ".cargo", "bin"),
    path.join(home, "bin"),
  ];
}

function pathEntries() {
  return (process.env.PATH || "")
    .split(path.delimiter)
    .filter(Boolean)
    .map((entry) => path.resolve(entry));
}

function pickUserBinDir() {
  const entries = new Set(pathEntries());
  const candidates = preferredBinDirs().map((entry) => path.resolve(entry));
  return candidates.find((entry) => entries.has(entry)) || candidates[0];
}

function binaryName() {
  return process.platform === "win32" ? "vera.exe" : "vera";
}

function shimName() {
  return process.platform === "win32" ? "vera.cmd" : "vera";
}

async function fetchText(url, redirects = 0) {
  const client = url.startsWith("https://") ? https : http;
  return new Promise((resolve, reject) => {
    const request = client.get(url, (response) => {
      const status = response.statusCode || 0;
      if ([301, 302, 303, 307, 308].includes(status) && response.headers.location) {
        if (redirects >= MAX_REDIRECTS) {
          reject(new Error(`too many redirects fetching ${url}`));
          return;
        }

        const nextUrl = new URL(response.headers.location, url).toString();
        resolve(fetchText(nextUrl, redirects + 1));
        return;
      }

      if (status < 200 || status >= 300) {
        reject(new Error(`request failed for ${url}: ${status}`));
        return;
      }

      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => resolve(body));
    });

    request.on("error", reject);
  });
}

async function downloadFile(url, destination, redirects = 0) {
  const client = url.startsWith("https://") ? https : http;
  await fsp.mkdir(path.dirname(destination), { recursive: true });

  return new Promise((resolve, reject) => {
    const request = client.get(url, (response) => {
      const status = response.statusCode || 0;
      if ([301, 302, 303, 307, 308].includes(status) && response.headers.location) {
        if (redirects >= MAX_REDIRECTS) {
          reject(new Error(`too many redirects downloading ${url}`));
          return;
        }

        const nextUrl = new URL(response.headers.location, url).toString();
        response.resume();
        resolve(downloadFile(nextUrl, destination, redirects + 1));
        return;
      }

      if (status < 200 || status >= 300) {
        reject(new Error(`download failed for ${url}: ${status}`));
        return;
      }

      const file = fs.createWriteStream(destination);
      response.pipe(file);
      file.on("finish", () => file.close(resolve));
      file.on("error", reject);
    });

    request.on("error", reject);
  });
}

async function sha256(filePath) {
  const hash = crypto.createHash("sha256");
  const stream = fs.createReadStream(filePath);

  return new Promise((resolve, reject) => {
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
    stream.on("error", reject);
  });
}

function runChecked(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

async function extractArchive(archivePath, destination) {
  await fsp.mkdir(destination, { recursive: true });
  if (archivePath.endsWith(".zip")) {
    runChecked("powershell", [
      "-NoProfile",
      "-Command",
      "Import-Module Microsoft.PowerShell.Archive; Expand-Archive",
      "-LiteralPath",
      archivePath,
      "-DestinationPath",
      destination,
      "-Force",
    ]);
    return;
  }

  runChecked("tar", ["-xzf", archivePath, "-C", destination]);
}

async function createShim(binaryPath) {
  const binDir = pickUserBinDir();
  await fsp.mkdir(binDir, { recursive: true });
  const shimPath = path.join(binDir, shimName());

  if (process.platform === "win32") {
    const contents = `@echo off\r\n"${binaryPath}" %*\r\n`;
    await fsp.writeFile(shimPath, contents, "utf8");
  } else {
    const contents = `#!/bin/sh\nexec "${binaryPath}" "$@"\n`;
    await fsp.writeFile(shimPath, contents, "utf8");
    await fsp.chmod(shimPath, 0o755);
  }

  return shimPath;
}

function isOnPath(dirPath) {
  return pathEntries().includes(path.resolve(dirPath));
}

async function loadManifest() {
  const primaryUrl = manifestUrl(packageVersion);
  try {
    const manifestText = await fetchText(primaryUrl);
    return JSON.parse(manifestText);
  } catch (error) {
    if (process.env.VERA_MANIFEST_URL) {
      throw error;
    }

    const fallbackText = await fetchText(latestManifestUrl());
    return JSON.parse(fallbackText);
  }
}

async function ensureBinaryInstalled() {
  const manifest = await loadManifest();
  const target = resolveTarget(process.platform, process.arch);
  const asset = manifest.assets && manifest.assets[target];
  if (!asset) {
    throw new Error(`no release asset for target ${target}`);
  }

  const version = manifest.version;
  const installDir = path.join(defaultVeraHome(), "bin", version, target);
  const binaryPath = path.join(installDir, binaryName());
  if (fs.existsSync(binaryPath)) {
    const stat = await fsp.stat(binaryPath);
    if (stat.size > 1_000_000) {
      await createShim(binaryPath);
      await writeInstallMetadata({
        installMethod: currentInstallMethod(),
        version,
        binaryPath,
        target,
      });
      return { binaryPath, version };
    }
    // Stale or broken file (e.g. leftover shim from interrupted install).
    await fsp.rm(binaryPath);
  }

  const tempRoot = await fsp.mkdtemp(path.join(os.tmpdir(), "vera-install-"));
  try {
    const archivePath = path.join(tempRoot, asset.archive);
    const extractDir = path.join(tempRoot, "extract");

    console.error(`Downloading Vera ${version} for ${target}...`);
    await downloadFile(asset.download_url, archivePath);

    const actualSha = await sha256(archivePath);
    if (actualSha !== asset.sha256) {
      throw new Error(`checksum mismatch for ${asset.archive}`);
    }

    await extractArchive(archivePath, extractDir);
    const extractedBinary = path.join(extractDir, `vera-${target}`, binaryName());
    await fsp.mkdir(installDir, { recursive: true });
    await fsp.copyFile(extractedBinary, binaryPath);
    if (process.platform !== "win32") {
      await fsp.chmod(binaryPath, 0o755);
    }

    const shimPath = await createShim(binaryPath);
    if (!isOnPath(path.dirname(shimPath))) {
      console.error(`Added Vera to ${path.dirname(shimPath)}. Add that directory to PATH to run \`vera\` directly.`);
    }

    await writeInstallMetadata({
      installMethod: currentInstallMethod(),
      version,
      binaryPath,
      target,
    });
    return { binaryPath, version };
  } finally {
    await fsp.rm(tempRoot, { recursive: true, force: true });
  }
}

function runBinary(binaryPath, args) {
  const result = spawnSync(binaryPath, args, { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  process.exit(result.status ?? 0);
}

async function main() {
  const { command, rest } = parseArgs(process.argv.slice(2));

  if (command === "_record-install-method") {
    await writeInstallMetadata({
      installMethod: "npm",
      version: packageVersion,
      binaryPath: null,
    });
    return;
  }

  const { binaryPath, version } = await ensureBinaryInstalled();

  if (command === "install") {
    console.error(`Vera ${version} installed.`);
    runBinary(binaryPath, ["agent", "install", ...rest]);
    return;
  }

  if (command === "help") {
    runBinary(binaryPath, ["--help"]);
    return;
  }

  runBinary(binaryPath, [command, ...rest]);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
