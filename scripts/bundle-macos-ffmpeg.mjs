import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertMacosDeployment } from "./macos-deployment.mjs";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const nativeRoot = path.join(repository, "src-tauri");
const run = (program, args) => execFileSync(program, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
const systemLibrary = (name) => name.startsWith("/usr/lib/") || name.startsWith("/System/Library/");

function dependencies(binary) {
  const ownIds = new Set(run("otool", ["-D", binary]).split("\n").map((line) => line.trim()).filter((line) => line && !line.endsWith(":")));
  return run("otool", ["-L", binary]).split("\n").slice(1)
    .map((line) => line.trim().replace(/\s+\(compatibility version.*$/, ""))
    .filter((line) => line && !line.endsWith(":") && !ownIds.has(line));
}

function rpaths(binary) {
  const lines = run("otool", ["-l", binary]).split("\n");
  const result = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() === "cmd LC_RPATH") {
      const match = lines[i + 2]?.trim().match(/^path (.*) \(offset \d+\)$/);
      if (match) result.push(match[1]);
    }
  }
  return result;
}

function verify(application) {
  const minimum = run("plutil", ["-extract", "LSMinimumSystemVersion", "raw", "-o", "-", path.join(application, "Contents/Info.plist")]);
  const executable = path.join(application, "Contents/MacOS/bili-shadowreplay");
  const directory = path.join(application, "Contents/Frameworks");
  if (!fs.existsSync(path.join(directory, "libvulkan.dylib"))) throw new Error("Bundled MoltenVK runtime is missing");
  const files = [executable, ...fs.readdirSync(directory).filter((name) => name.endsWith(".dylib")).map((name) => path.join(directory, name))];
  for (const file of files) {
    assertMacosDeployment(run("otool", ["-l", file]), minimum, file);
    for (const dependency of dependencies(file)) {
      if (systemLibrary(dependency)) continue;
      const location = dependency.startsWith("@loader_path/")
        ? path.resolve(path.dirname(file), dependency.slice("@loader_path/".length))
        : dependency.startsWith("@executable_path/")
          ? path.resolve(path.dirname(executable), dependency.slice("@executable_path/".length))
          : null;
      if (!location || !location.startsWith(path.resolve(application) + path.sep) || !fs.existsSync(location)) {
        throw new Error(`Unbundled library in ${file}: ${dependency}`);
      }
    }
  }
  console.log(`Verified ${files.length - 1} bundled native libraries`);
}

if (process.argv[2] === "--verify") {
  verify(path.resolve(process.argv[3]));
} else {
  if (process.platform !== "darwin") throw new Error("macOS bundling requires otool, install_name_tool and codesign");
  const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
  const target = process.env.CARGO_TARGET_DIR ? path.resolve(nativeRoot, process.env.CARGO_TARGET_DIR) : path.join(nativeRoot, "target");
  const triple = process.env.RUST_TARGET || process.env.TAURI_ENV_TARGET_TRIPLE;
  const requestedArch = triple?.startsWith("aarch64-") ? "arm64" : triple?.startsWith("x86_64-") ? "x86_64" : null;
  const hostArch = process.arch === "arm64" ? "arm64" : "x86_64";
  // Tauri sets TARGET_TRIPLE even without --target, when Cargo uses target/release.
  // CI sets RUST_TARGET explicitly; a cross-architecture target must never fall
  // back to an unrelated native binary left by an earlier build.
  const candidates = process.env.RUST_TARGET || (requestedArch && requestedArch !== hostArch)
    ? [path.join(target, triple, profile, "bili-shadowreplay")]
    : [triple && path.join(target, triple, profile, "bili-shadowreplay"), path.join(target, profile, "bili-shadowreplay")].filter(Boolean);
  // --binary/--runtime are useful for a small native link smoke test too.
  const binaryFlag = process.argv.indexOf("--binary");
  const runtimeFlag = process.argv.indexOf("--runtime");
  const minimumFlag = process.argv.indexOf("--minimum-system-version");
  if (minimumFlag >= 0 && (binaryFlag < 0 || runtimeFlag < 0)) throw new Error("Minimum system version override is only available with --binary and --runtime for smoke tests");
  const config = JSON.parse(fs.readFileSync(path.join(nativeRoot, "tauri.macos.conf.json"), "utf8"));
  const minimum = minimumFlag >= 0 ? process.argv[minimumFlag + 1] : config.bundle.macOS.minimumSystemVersion;
  const existing = candidates.filter((candidate) => fs.existsSync(candidate));
  if (binaryFlag < 0 && existing.length > 1) throw new Error("Both native and explicit-target executables exist; set RUST_TARGET to the --target used for this build");
  const executable = binaryFlag >= 0 ? path.resolve(process.argv[binaryFlag + 1]) : existing[0];
  if (!executable || !fs.existsSync(executable)) throw new Error(`Application executable not found: ${candidates.join(", ")}`);
  const assertArchitecture = (file) => {
    if (requestedArch && !run("lipo", ["-archs", file]).split(/\s+/).includes(requestedArch)) throw new Error(`Wrong architecture in ${file}; expected ${requestedArch}`);
  };
  assertArchitecture(executable);
  assertMacosDeployment(run("otool", ["-l", executable]), minimum, executable);
  const destination = runtimeFlag >= 0 ? path.resolve(process.argv[runtimeFlag + 1]) : path.join(nativeRoot, "target/ffmpeg-runtime");
  const temporary = fs.mkdtempSync(destination + ".");
  const libraries = new Map();
  const names = new Map();
  const executablePaths = rpaths(executable);
  const expand = (value, owner) => value.replace(/^@loader_path(?=\/|$)/, path.dirname(owner)).replace(/^@executable_path(?=\/|$)/, path.dirname(executable));
  function resolve(dependency, owner) {
    const candidates = dependency.startsWith("@rpath/")
      ? [...rpaths(owner).map((base) => expand(base, owner)), ...executablePaths.map((base) => expand(base, executable))].map((base) => path.join(base, dependency.slice(7)))
      : [expand(dependency, owner)];
    // A previous bundle invocation may already have relocated the Cargo binary.
    candidates.push(path.join(destination, path.basename(dependency)));
    const found = candidates.find((candidate) => path.isAbsolute(candidate) && fs.existsSync(candidate));
    if (!found) throw new Error(`Cannot resolve ${dependency} referenced by ${owner}`);
    return fs.realpathSync(found);
  }
  function collect(owner) {
    const links = [];
    for (const dependency of dependencies(owner)) {
      if (systemLibrary(dependency)) continue;
      if (/^(?:libvulkan(?:\.[\d.]+)?|libMoltenVK)\.dylib$/.test(path.basename(dependency))) {
        // libplacebo may link the loader even though FFmpeg opens it at
        // runtime. Route both to the same MoltenVK implementation; mixing a
        // loader's handles with direct MoltenVK dispatch is invalid.
        links.push({ dependency, name: "libvulkan.dylib" });
        continue;
      }
      const source = resolve(dependency, owner);
      if (source === fs.realpathSync(owner)) continue; // LC_ID_DYLIB
      const name = path.basename(source);
      if (names.has(name) && names.get(name) !== source) throw new Error(`Conflicting library basename: ${name}`);
      links.push({ dependency, name });
      if (!libraries.has(source)) {
        assertArchitecture(source);
        assertMacosDeployment(run("otool", ["-l", source]), minimum, source);
        names.set(name, source);
        libraries.set(source, null);
        libraries.set(source, { name, links: collect(source) });
      }
    }
    return links;
  }
  try {
    const executableLinks = collect(executable);
    if (![...names.keys()].some((name) => /^libavformat\./.test(name))) throw new Error("Executable does not link libavformat; refusing an incomplete FFmpeg bundle");
    // FFmpeg opens libvulkan.dylib first. Ship MoltenVK under that name so it
    // can be preloaded by absolute app path, with no loader/ICD/environment
    // dependency and no possibility of selecting a different installed ICD.
    const moltenCandidates = [process.env.BSR_MOLTENVK_LIBRARY,
      process.env.VULKAN_SDK && path.join(process.env.VULKAN_SDK, "lib/libMoltenVK.dylib"),
      "/opt/homebrew/opt/molten-vk/lib/libMoltenVK.dylib", "/usr/local/opt/molten-vk/lib/libMoltenVK.dylib",
      path.join(destination, "libvulkan.dylib")].filter(Boolean);
    const molten = moltenCandidates.find((candidate) => fs.existsSync(candidate));
    if (!molten) throw new Error("MoltenVK runtime missing: install molten-vk or set BSR_MOLTENVK_LIBRARY");
    const moltenSource = fs.realpathSync(molten);
    assertArchitecture(moltenSource);
    assertMacosDeployment(run("otool", ["-l", moltenSource]), minimum, moltenSource);
    if (!run("nm", ["-gU", moltenSource]).includes("_vkExportMetalObjectsEXT")) throw new Error("MoltenVK must export VK_EXT_metal_objects");
    if (names.has("libvulkan.dylib")) throw new Error("Executable already links a competing Vulkan loader");
    libraries.set(moltenSource, { name: "libvulkan.dylib", links: collect(moltenSource) });
    for (const [source, { name: libraryName, links }] of libraries) {
      const copy = path.join(temporary, libraryName);
      fs.copyFileSync(source, copy);
      fs.chmodSync(copy, 0o755);
      // dyld matches a preloaded @rpath ID through dlopen's bare filename;
      // @loader_path IDs do not match calls originating in another image.
      const changes = ["-id", `${libraryName === "libvulkan.dylib" ? "@rpath" : "@loader_path"}/${libraryName}`];
      for (const { dependency, name } of links) changes.push("-change", dependency, `@loader_path/${name}`);
      run("install_name_tool", [...changes, copy]);
      const identity = process.env.APPLE_SIGNING_IDENTITY || config.bundle.macOS.signingIdentity || "-";
      run("codesign", ["--force", "--sign", identity, copy]);
    }
    fs.rmSync(destination, { recursive: true, force: true });
    fs.renameSync(temporary, destination);
    for (const { dependency, name } of executableLinks) run("install_name_tool", ["-change", dependency, `@executable_path/../Frameworks/${name}`, executable]);
    console.log(`Prepared ${libraries.size} native libraries for ${executable}`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}
