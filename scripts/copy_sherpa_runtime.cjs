const fs = require("node:fs");
const path = require("node:path");

const repositoryRoot = path.resolve(__dirname, "..");
const nativeRoot = path.join(repositoryRoot, "src-tauri");
// Tauri invokes Cargo from the native project directory. Resolve a relative
// CARGO_TARGET_DIR there too, independently of this hook's working directory.
const targetDirectory = process.env.CARGO_TARGET_DIR
  ? path.resolve(nativeRoot, process.env.CARGO_TARGET_DIR)
  : path.join(nativeRoot, "target");
const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
const sourceDirectory = path.join(targetDirectory, profile);
const destinationDirectory = path.join(
  repositoryRoot,
  "src-tauri",
  "windows-runtime"
);

const runtimeDlls = [
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "sherpa-onnx-c-api.dll",
  "sherpa-onnx-cxx-api.dll",
];

for (const dll of runtimeDlls) {
  const source = path.join(sourceDirectory, dll);
  if (!fs.existsSync(source)) {
    throw new Error(`Required sherpa-onnx runtime DLL not found: ${source}`);
  }

  fs.copyFileSync(source, path.join(destinationDirectory, dll));
}
