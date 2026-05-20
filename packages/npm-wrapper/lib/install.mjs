import { createHash } from "node:crypto";
import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { chmod, rename } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pipeline } from "node:stream/promises";
import { createGunzip } from "node:zlib";
import { extract as tarExtract } from "tar";
import AdmZip from "adm-zip";

const REPO = "eastreams/loong";
const DEFAULT_TAG = "v0.1.2-alpha.2";
const PACKAGE_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "..");
const CACHE_ROOT = join(PACKAGE_ROOT, ".bin");
const BIN_DIRECTORY = join(CACHE_ROOT, "current");

function detectAssetTarget() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "darwin" && arch === "arm64") {
    return {
      label: "macos-arm64",
      binaryName: "loong",
      targetTriple: "aarch64-apple-darwin"
    };
  }

  if (platform === "darwin" && arch === "x64") {
    return {
      label: "macos-x64",
      binaryName: "loong",
      targetTriple: "x86_64-apple-darwin"
    };
  }

  if (platform === "linux" && arch === "arm64") {
    return {
      label: "linux-arm64-gnu",
      binaryName: "loong",
      targetTriple: "aarch64-unknown-linux-gnu"
    };
  }

  if (platform === "linux" && arch === "x64") {
    const requestedLibc = process.env.LOONG_NPM_TARGET_LIBC?.trim();
    if (requestedLibc === "musl") {
      return {
        label: "linux-x64-musl",
        binaryName: "loong",
        targetTriple: "x86_64-unknown-linux-musl"
      };
    }

    return {
      label: "linux-x64-gnu",
      binaryName: "loong",
      targetTriple: "x86_64-unknown-linux-gnu"
    };
  }

  if (platform === "win32" && arch === "x64") {
    return {
      label: "windows-x64",
      binaryName: "loong.exe",
      targetTriple: "x86_64-pc-windows-msvc"
    };
  }

  throw new Error(`unsupported npm wrapper target: ${platform}/${arch}`);
}

function resolveReleaseTag() {
  const envTag = process.env.LOONG_NPM_RELEASE_TAG;
  if (envTag && envTag.trim() !== "") {
    return envTag.trim();
  }

  return DEFAULT_TAG;
}

function archiveExtensionForCurrentPlatform() {
  return process.platform === "win32" ? "zip" : "tar.gz";
}

function archiveNameFor(tag, targetLabel) {
  const extension = archiveExtensionForCurrentPlatform();
  return `loong-${tag}-${targetLabel}.${extension}`;
}

function legacyArchiveNameFor(tag, target) {
  const extension = archiveExtensionForCurrentPlatform();
  return `loong-${tag}-${target}.${extension}`;
}

function checksumManifestNameFor(tag) {
  return `loong-${tag}-SHA256SUMS.txt`;
}

function legacyChecksumNameFor(archiveName) {
  return `${archiveName}.sha256`;
}

function releaseBaseUrlFor(tag) {
  return `https://github.com/${REPO}/releases/download/${tag}`;
}

function resolveBinaryPath(binaryName) {
  return join(BIN_DIRECTORY, binaryName);
}

function installedBinaryIsUsable(binaryPath) {
  if (!existsSync(binaryPath)) {
    return false;
  }

  const stats = statSync(binaryPath);
  return stats.isFile() && stats.size > 0;
}

function parseChecksumManifest(manifestText, archiveName) {
  const lines = manifestText.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed === "") {
      continue;
    }

    const match = trimmed.match(/^([0-9a-fA-F]+)\s+(.+)$/);
    if (!match) {
      continue;
    }

    const digest = match[1].toLowerCase();
    const fileName = match[2];
    if (fileName === archiveName) {
      return digest;
    }
  }

  throw new Error(`checksum manifest did not contain ${archiveName}`);
}

function parseLegacyChecksumFile(checksumText, archiveName) {
  const trimmed = checksumText.trim();
  const match = trimmed.match(/^([0-9a-fA-F]+)\s+(.+)$/);
  if (!match) {
    throw new Error(`legacy checksum file did not contain a digest for ${archiveName}`);
  }

  const digest = match[1].toLowerCase();
  const fileName = match[2];
  if (fileName !== archiveName) {
    throw new Error(`legacy checksum file referenced ${fileName} instead of ${archiveName}`);
  }

  return digest;
}

function sha256File(filePath) {
  const hash = createHash("sha256");
  const content = readFileSync(filePath);
  hash.update(content);
  return hash.digest("hex");
}

async function downloadToFile(url, filePath) {
  const response = await fetch(url, {
    headers: {
      "user-agent": "@eastream/loong"
    }
  });

  if (!response.ok || !response.body) {
    throw new Error(`download failed for ${url}: ${response.status} ${response.statusText}`);
  }

  await pipeline(response.body, createWriteStream(filePath));
}

async function downloadFirstAvailableAsset({ baseUrl, candidateNames, outputDirectory }) {
  const errors = [];

  for (const candidateName of candidateNames) {
    const candidatePath = join(outputDirectory, candidateName);
    const candidateUrl = `${baseUrl}/${candidateName}`;

    try {
      await downloadToFile(candidateUrl, candidatePath);
      return {
        archiveName: candidateName,
        archivePath: candidatePath
      };
    } catch (error) {
      errors.push(`${candidateName}: ${error.message}`);
    }
  }

  throw new Error(`download failed for all candidate assets: ${errors.join("; ")}`);
}

async function resolveExpectedDigest({ baseUrl, archiveName, workingDirectory, tag }) {
  const manifestName = checksumManifestNameFor(tag);
  const legacyChecksumName = legacyChecksumNameFor(archiveName);
  const manifestPath = join(workingDirectory, manifestName);
  const legacyChecksumPath = join(workingDirectory, legacyChecksumName);

  try {
    await downloadToFile(`${baseUrl}/${manifestName}`, manifestPath);
    const manifestText = readFileSync(manifestPath, "utf8");
    return parseChecksumManifest(manifestText, archiveName);
  } catch (error) {
    await downloadToFile(`${baseUrl}/${legacyChecksumName}`, legacyChecksumPath);
    const legacyChecksumText = readFileSync(legacyChecksumPath, "utf8");
    return parseLegacyChecksumFile(legacyChecksumText, archiveName);
  }
}

async function extractTarGz(archivePath, outputDirectory) {
  mkdirSync(outputDirectory, { recursive: true });
  await pipeline(
    createReadStream(archivePath),
    createGunzip(),
    tarExtract({ cwd: outputDirectory })
  );
}

async function extractZip(archivePath, outputDirectory) {
  mkdirSync(outputDirectory, { recursive: true });
  const zip = new AdmZip(archivePath);
  zip.extractAllTo(outputDirectory, true);
}

async function extractArchive(archivePath, outputDirectory) {
  if (archivePath.endsWith(".tar.gz")) {
    await extractTarGz(archivePath, outputDirectory);
    return;
  }

  if (archivePath.endsWith(".zip")) {
    await extractZip(archivePath, outputDirectory);
    return;
  }

  throw new Error(`unsupported archive format: ${archivePath}`);
}

async function installBinaryRelease() {
  const target = detectAssetTarget();
  const tag = resolveReleaseTag();
  const preferredArchiveName = archiveNameFor(tag, target.label);
  const legacyArchiveName = legacyArchiveNameFor(tag, target.targetTriple);
  const baseUrl = releaseBaseUrlFor(tag);
  const workingDirectory = join(tmpdir(), `loong-npm-${Date.now()}`);
  const extractDirectory = join(workingDirectory, "extract");
  const binaryPath = resolveBinaryPath(target.binaryName);

  mkdirSync(workingDirectory, { recursive: true });
  mkdirSync(CACHE_ROOT, { recursive: true });

  try {
    const archive = await downloadFirstAvailableAsset({
      baseUrl,
      candidateNames: [preferredArchiveName, legacyArchiveName],
      outputDirectory: workingDirectory
    });
    const expectedDigest = await resolveExpectedDigest({
      baseUrl,
      archiveName: archive.archiveName,
      workingDirectory,
      tag
    });
    const actualDigest = sha256File(archive.archivePath);

    if (expectedDigest !== actualDigest) {
      throw new Error(`checksum mismatch for ${archive.archiveName}`);
    }

    rmSync(BIN_DIRECTORY, { recursive: true, force: true });
    mkdirSync(BIN_DIRECTORY, { recursive: true });
    await extractArchive(archive.archivePath, extractDirectory);

    const extractedBinaryPath = join(extractDirectory, target.binaryName);
    mkdirSync(dirname(binaryPath), { recursive: true });
    await rename(extractedBinaryPath, binaryPath);

    if (process.platform !== "win32") {
      await chmod(binaryPath, 0o755);
    }

    return binaryPath;
  } finally {
    rmSync(workingDirectory, { recursive: true, force: true });
  }
}

export async function ensureInstalledBinary({ allowDownload }) {
  const target = detectAssetTarget();
  const binaryPath = resolveBinaryPath(target.binaryName);

  if (installedBinaryIsUsable(binaryPath)) {
    return binaryPath;
  }

  if (!allowDownload) {
    throw new Error("loong binary is not installed yet");
  }

  return installBinaryRelease();
}
