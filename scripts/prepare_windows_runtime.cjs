const fs = require("node:fs");
const path = require("node:path");

fs.mkdirSync(
  path.resolve(__dirname, "..", "src-tauri", "windows-runtime"),
  { recursive: true }
);
