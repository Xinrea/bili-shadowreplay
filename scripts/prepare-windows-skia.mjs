#!/usr/bin/env node
// Prepare a Windows Skia archive whose bundled ICU cannot clash with V8's.
//
// skia-bindings' prebuilt Windows `skunicode_icu.lib` and rusty_v8 both embed
// ICU 74. Skia's copy is built without ICU's version renaming, and a few
// ICU-internal names are never renamed in V8's copy either, so both define
// them. MSVC refuses the duplicate (`ulocimp_getParent`, LNK2005); the others
// would bind Skia's ICU to V8's functions. Renaming them in Skia's library, at
// their definitions and uses alike, keeps each ICU on its own code.
//
// The patched archive is served to skia-bindings through SKIA_BINARIES_URL.
// See docs/development/native-overlay.md.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Must match the skia-bindings version pinned through native-overlay.
const SKIA_BINDINGS_VERSION = "0.82.0";
const KEY_PREFIX = "7ca7f6be2332d3c9f3ad-x86_64-pc-windows-msvc-";
const ARCHIVES = {
  textlayout: "300dcfe6ed498b37b5f3c0ef4ac6b8f23e4f6cf5cd603cd28c3139ca7de29eaf",
  "textlayout-vulkan": "2d483906db2c7ee55983605ea71a6ceecf64329c6e1586276b10e38870fd1fbc",
};

// Every unversioned symbol that Skia's ICU and rusty_v8 137's ICU both define.
const SHARED_ICU_SYMBOLS = [
  "ulocimp_getParent",
  "uprv_detectWindowsTimeZone",
  "uprv_getWindowsTimeZoneInfo",
  "icudt74_dat",
  "??0UCaseMap@@QEAA@PEBDIPEAW4UErrorCode@@@Z",
  "??1UCaseMap@@QEAA@XZ",
  "??1CollatorSpec@@QEAA@XZ",
  "?reg@CReg@@SAPEBXPEB_SPEBDPEAW4UErrorCode@@@Z",
];
const PREFIX = "bsr_skia_";

const root = resolve(import.meta.dirname, "..");

export function parseOptions(args) {
  let features = "textlayout";
  let directory;
  for (const arg of args) {
    if (arg === "--vulkan") features = "textlayout-vulkan";
    else if (arg.startsWith("-")) throw new Error(`Unknown option: ${arg}`);
    else if (directory !== undefined) throw new Error("Only one output directory may be provided");
    else directory = arg;
  }
  const key = KEY_PREFIX + features;
  const archive = `skia-binaries-${key}.tar.gz`;
  return {
    features,
    key,
    archive,
    url: `https://github.com/rust-skia/skia-binaries/releases/download/${SKIA_BINDINGS_VERSION}/${archive}`,
    sha256: ARCHIVES[features],
    outputDir: resolve(directory ?? join(root, "src-tauri", "target", "skia-binaries")),
  };
}

export function checkArchiveIdentity(directory, options) {
  const key = readFileSync(join(directory, "key.txt"), "utf8").trim();
  const tag = readFileSync(join(directory, "tag.txt"), "utf8").trim();
  if (key !== options.key || tag !== SKIA_BINDINGS_VERSION) {
    throw new Error(`Skia archive identifies ${tag}/${key}, expected ${SKIA_BINDINGS_VERSION}/${options.key}`);
  }
}

function checkLockedVersion() {
  const lock = readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8");
  // Windows checkouts may use CRLF.
  const locked = /name = "skia-bindings"\r?\nversion = "([^"]+)"/.exec(lock)?.[1];
  if (locked !== SKIA_BINDINGS_VERSION) {
    throw new Error(`Cargo.lock has skia-bindings ${locked}, but this script patches ${SKIA_BINDINGS_VERSION}. `
      + "Update ARCHIVES, KEY_PREFIX and SHARED_ICU_SYMBOLS for the new version.");
  }
}

function findObjcopy() {
  const exe = process.platform === "win32" ? ".exe" : "";
  const sysroot = execFileSync("rustc", ["--print", "sysroot"], { encoding: "utf8" }).trim();
  const host = /^host: (.+)$/m.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
  const bin = join(sysroot, "lib", "rustlib", host, "bin");
  const candidates = [
    process.env.LLVM_OBJCOPY,
    join(bin, `llvm-objcopy${exe}`),
    join(bin, `rust-objcopy${exe}`),
  ].filter(Boolean);
  const found = candidates.find((path) => existsSync(path));
  if (found) return found;
  try {
    execFileSync("llvm-objcopy", ["--version"], { stdio: "ignore" });
    return "llvm-objcopy";
  } catch {
    throw new Error("llvm-objcopy not found. Run `rustup component add llvm-tools`, or set LLVM_OBJCOPY.");
  }
}

async function download(options) {
  const response = await fetch(options.url);
  if (!response.ok) throw new Error(`Downloading ${options.url} failed: HTTP ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  const sha256 = createHash("sha256").update(data).digest("hex");
  if (sha256 !== options.sha256) throw new Error(`${options.archive} has SHA-256 ${sha256}, expected ${options.sha256}`);
  return data;
}

// The names sit NUL-terminated in the archive index and the string tables.
const contains = (data, name) => data.includes(Buffer.from(`\0${name}\0`, "latin1"));

async function main() {
  const options = parseOptions(process.argv.slice(2));
  const { outputDir, archive } = options;
  checkLockedVersion();
  const objcopy = findObjcopy();
  // CPU and Vulkan archives can coexist in the same cache directory.
  const work = join(outputDir, `work-${options.features}`);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });

  writeFileSync(join(work, archive), await download(options));
  // Relative paths only: Git for Windows' GNU tar reads `D:\...` as a remote host.
  execFileSync("tar", ["-xzf", archive], { cwd: work, stdio: "inherit" });
  checkArchiveIdentity(join(work, "skia-binaries"), options);

  const lib = join(work, "skia-binaries", "skunicode_icu.lib");
  const patched = `${lib}.patched`;
  const missing = SHARED_ICU_SYMBOLS.filter((name) => !contains(readFileSync(lib), name));
  if (missing.length) throw new Error(`skunicode_icu.lib no longer defines: ${missing.join(", ")}`);
  execFileSync(objcopy, [
    ...SHARED_ICU_SYMBOLS.map((name) => `--redefine-sym=${name}=${PREFIX}${name}`),
    lib,
    patched,
  ], { stdio: "inherit" });
  const result = readFileSync(patched);
  const unrenamed = SHARED_ICU_SYMBOLS.filter((name) => contains(result, name) || !contains(result, PREFIX + name));
  if (unrenamed.length) throw new Error(`llvm-objcopy did not rename: ${unrenamed.join(", ")}`);
  renameSync(patched, lib);

  rmSync(join(work, archive));
  execFileSync("tar", ["-czf", archive, "skia-binaries"], { cwd: work, stdio: "inherit" });
  renameSync(join(work, archive), join(outputDir, archive));
  rmSync(work, { recursive: true, force: true });

  // skia-bindings strips `file://` and reads the rest as a path, so Windows
  // paths go in as `file://D:/...`. It fills in `{key}`; an unexpected key
  // finds no archive and falls back to building Skia, unpatched, from source.
  const url = `file://${outputDir.replaceAll("\\", "/")}/skia-binaries-{key}.tar.gz`;
  if (process.env.GITHUB_ENV) appendFileSync(process.env.GITHUB_ENV, `SKIA_BINARIES_URL=${url}\n`);
  console.log(`Patched ${SHARED_ICU_SYMBOLS.length} ICU symbols in ${join(outputDir, archive)}`);
  console.log(`SKIA_BINARIES_URL=${url}`);
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
