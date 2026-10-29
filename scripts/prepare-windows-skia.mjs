#!/usr/bin/env node
// Prepare a Windows Skia archive whose private symbols cannot clash with V8's.
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
import { appendFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

// Must match the skia-bindings version pinned through native-overlay.
const SKIA_BINDINGS_VERSION = "0.82.0";
const args = process.argv.slice(2);
const vulkan = args.includes("--vulkan");
const KEY = `7ca7f6be2332d3c9f3ad-x86_64-pc-windows-msvc-textlayout${vulkan ? "-vulkan" : ""}`;
const ARCHIVE = `skia-binaries-${KEY}.tar.gz`;
const ARCHIVE_URL = `https://github.com/rust-skia/skia-binaries/releases/download/${SKIA_BINDINGS_VERSION}/${ARCHIVE}`;
const ARCHIVE_SHA256 = vulkan
  ? "2d483906db2c7ee55983605ea71a6ceecf64329c6e1586276b10e38870fd1fbc"
  : "300dcfe6ed498b37b5f3c0ef4ac6b8f23e4f6cf5cd603cd28c3139ca7de29eaf";

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
const outputDir = resolve(args.find((arg) => arg !== "--vulkan") ?? join(root, "src-tauri", "target", "skia-binaries"));

function checkLockedVersion() {
  const lock = readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8");
  // Windows checkouts may use CRLF.
  const locked = /name = "skia-bindings"\r?\nversion = "([^"]+)"/.exec(lock)?.[1];
  if (locked !== SKIA_BINDINGS_VERSION) {
    throw new Error(`Cargo.lock has skia-bindings ${locked}, but this script patches ${SKIA_BINDINGS_VERSION}. `
      + "Update KEY, ARCHIVE_SHA256 and SHARED_ICU_SYMBOLS for the new version.");
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

async function download() {
  const response = await fetch(ARCHIVE_URL);
  if (!response.ok) throw new Error(`Downloading ${ARCHIVE_URL} failed: HTTP ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  const sha256 = createHash("sha256").update(data).digest("hex");
  if (sha256 !== ARCHIVE_SHA256) throw new Error(`${ARCHIVE} has SHA-256 ${sha256}, expected ${ARCHIVE_SHA256}`);
  return data;
}

// The names sit NUL-terminated in the archive index and the string tables.
const contains = (data, name) => data.includes(Buffer.from(`\0${name}\0`, "latin1"));

async function main() {
  checkLockedVersion();
  const objcopy = findObjcopy();
  const work = join(outputDir, "work");
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });

  writeFileSync(join(work, ARCHIVE), await download());
  // Relative paths only: Git for Windows' GNU tar reads `D:\...` as a remote host.
  execFileSync("tar", ["-xzf", ARCHIVE], { cwd: work, stdio: "inherit" });

  for (const [library, symbols] of [
    ["skunicode_icu.lib", SHARED_ICU_SYMBOLS],
    // Skia's _HAS_EXCEPTIONS=0 STL defines terminate() as abort(), while
    // V8's libc++ defines the same symbol with its own handler semantics.
    ["skia.lib", ["?terminate@std@@YAXXZ"]],
  ]) {
    const lib = join(work, "skia-binaries", library);
    const patched = `${lib}.patched`;
    const missing = symbols.filter((name) => !contains(readFileSync(lib), name));
    if (missing.length) throw new Error(`${library} no longer defines: ${missing.join(", ")}`);
    execFileSync(objcopy, [
      ...symbols.map((name) => `--redefine-sym=${name}=${PREFIX}${name}`),
      lib,
      patched,
    ], { stdio: "inherit" });
    const result = readFileSync(patched);
    const unrenamed = symbols.filter((name) => contains(result, name) || !contains(result, PREFIX + name));
    if (unrenamed.length) throw new Error(`llvm-objcopy did not rename: ${unrenamed.join(", ")}`);
    renameSync(patched, lib);
  }

  rmSync(join(work, ARCHIVE));
  execFileSync("tar", ["-czf", ARCHIVE, "skia-binaries"], { cwd: work, stdio: "inherit" });
  renameSync(join(work, ARCHIVE), join(outputDir, ARCHIVE));
  rmSync(work, { recursive: true, force: true });

  // skia-bindings strips `file://` and reads the rest as a path, so Windows
  // paths go in as `file://D:/...`. It fills in `{key}`; a key other than KEY
  // finds no archive and falls back to building Skia, unpatched, from source.
  const url = `file://${outputDir.replaceAll("\\", "/")}/skia-binaries-{key}.tar.gz`;
  if (process.env.GITHUB_ENV) appendFileSync(process.env.GITHUB_ENV, `SKIA_BINARIES_URL=${url}\n`);
  console.log(`Isolated ICU and STL symbols in ${join(outputDir, ARCHIVE)}`);
  console.log(`SKIA_BINARIES_URL=${url}`);
}

await main();
