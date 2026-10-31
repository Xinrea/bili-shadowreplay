import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { prepareWindowsSkia } from "./prepare-windows-skia.mjs";

// Windows is case-insensitive, but Node's child-process environment object is
// not: duplicate Path/PATH keys can silently drop the configured DLL path.
export function windowsEnvironment(env) {
  return Object.fromEntries(Object.entries(env).map(([key, value]) => [key.toUpperCase(), value]));
}

export function parseDeveloperEnvironment(output, base) {
  const env = windowsEnvironment(base);
  for (const line of output.split(/\r?\n/)) {
    const equal = line.indexOf("=");
    if (equal > 0) env[line.slice(0, equal).toUpperCase()] = line.slice(equal + 1);
  }
  return env;
}

const run = (program, args, env) => execFileSync(program, args, {
  env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
}).trim();

function visualStudioEnvironment(env) {
  const vswhere = path.join(env["PROGRAMFILES(X86)"] || "C:\\Program Files (x86)", "Microsoft Visual Studio/Installer/vswhere.exe");
  // Preserve a deliberately selected developer toolchain (e.g. CUDA's
  // supported VS version) instead of switching it to the newest installation.
  let installation = env.VSINSTALLDIR;
  if (!installation || !fs.existsSync(path.join(installation, "Common7/Tools/VsDevCmd.bat"))) {
    if (!fs.existsSync(vswhere)) throw new Error("Install Visual Studio 2022 or Build Tools with Desktop development with C++ (MSVC, Windows SDK and CMake tools).");
    installation = run(vswhere, ["-utf8", "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], env);
  }
  if (!installation) throw new Error("Visual Studio's Desktop development with C++ workload is missing (MSVC x64 and Windows SDK required).");
  const script = path.join(installation, "Common7/Tools/VsDevCmd.bat");
  // Only the child environment changes; no Developer PowerShell profile or
  // persistent user/system PATH edits are needed.
  const output = execFileSync(env.COMSPEC || "cmd.exe", ["/d", "/u", "/s", "/c", `"call "${script}" -no_logo -arch=x64 -host_arch=x64 >nul && set"`], {
    env, encoding: "utf16le", windowsVerbatimArguments: true, stdio: ["ignore", "pipe", "pipe"],
  });
  return { env: parseDeveloperEnvironment(output, env), installation };
}

function llvmEnvironment(env, installation) {
  const candidates = env.LIBCLANG_PATH ? [env.LIBCLANG_PATH] : [
    path.join(env.PROGRAMFILES || "C:\\Program Files", "LLVM/bin"),
    path.join(installation, "VC/Tools/Llvm/x64/bin"),
    path.join(installation, "VC/Tools/Llvm/bin"),
    ...(env.PATH || "").split(path.delimiter),
  ];
  const directory = candidates.find((candidate) => fs.existsSync(path.join(candidate, "libclang.dll")));
  if (!directory) throw new Error("LLVM libclang.dll is missing. Install 64-bit LLVM or Visual Studio's C++ Clang tools; no LIBCLANG_PATH configuration is needed.");
  const configured = { ...env, LIBCLANG_PATH: path.resolve(directory) };
  const objcopy = path.join(directory, "llvm-objcopy.exe");
  if (!configured.LLVM_OBJCOPY && fs.existsSync(objcopy)) configured.LLVM_OBJCOPY = objcopy;
  // Standalone LLVM and VS CMake/Ninja tools are now discoverable even when
  // their installers were not asked to modify the user's PATH.
  const cmake = path.join(installation, "Common7/IDE/CommonExtensions/Microsoft/CMake");
  const directories = [directory, path.join(cmake, "CMake/bin"), path.join(cmake, "Ninja")].filter((candidate) => fs.existsSync(candidate));
  configured.PATH = [...directories, env.PATH].join(path.delimiter);
  return configured;
}

export async function prepareWindowsEnvironment(root, initial = process.env) {
  let env = windowsEnvironment(initial);
  const host = /^host: (.+)$/m.exec(run("rustc", ["-vV"], env))?.[1].trim();
  if (host !== "x86_64-pc-windows-msvc") throw new Error(`Windows native exports require the x86_64-pc-windows-msvc Rust toolchain; current host: ${host}`);
  const studio = visualStudioEnvironment(env);
  env = llvmEnvironment(studio.env, studio.installation);
  const target = path.join(root, "src-tauri/target");
  fs.mkdirSync(target, { recursive: true });
  const temporary = fs.mkdtempSync(path.join(target, ".native-env-"));
  try {
    const receipt = path.join(temporary, "environment.json");
    execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", path.join(root, "ffmpeg_setup.ps1"), "-EnvironmentFile", receipt], { env, stdio: "inherit" });
    env = { ...env, ...JSON.parse(fs.readFileSync(receipt, "utf8").replace(/^\uFEFF/, "")) };
  } finally { fs.rmSync(temporary, { recursive: true, force: true }); }
  if (!env.SKIA_BINARIES_URL) env.SKIA_BINARIES_URL = await prepareWindowsSkia(undefined, env);
  console.log(`[native] Visual Studio: ${studio.installation}`);
  console.log(`[native] LLVM: ${env.LIBCLANG_PATH}`);
  console.log(`[native] Skia: ${env.SKIA_BINARIES_URL}`);
  return env;
}
