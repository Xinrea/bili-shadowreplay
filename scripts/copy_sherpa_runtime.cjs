const fs = require("node:fs");
const path = require("node:path");

const repositoryRoot = path.resolve(__dirname, "..");
const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
const sourceDirectory = path.join(repositoryRoot, "src-tauri", "target", profile);
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
