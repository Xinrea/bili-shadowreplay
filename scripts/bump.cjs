#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const packageJsonPath = path.join(process.cwd(), "package.json");
const cargoTomlPath = path.join(process.cwd(), "src-tauri", "Cargo.toml");
const cargoLockPath = path.join(process.cwd(), "src-tauri", "Cargo.lock");

function readPackageVersion() {
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  return packageJson.version;
}

function updatePackageJson(version) {
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  if (packageJson.version === version) return;
  packageJson.version = version;
  fs.writeFileSync(
    packageJsonPath,
    JSON.stringify(packageJson, null, 2) + "\n"
  );
  console.log(`✅ Updated package.json version to ${version}`);
}

function updateCargoToml(version) {
  let cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
  // Only touch the version in the [package] section (the first match).
  cargoToml = cargoToml.replace(/^version = ".*"$/m, `version = "${version}"`);
  fs.writeFileSync(cargoTomlPath, cargoToml);
  console.log(`✅ Updated Cargo.toml version to ${version}`);
}

function updateCargoLock(version) {
  if (!fs.existsSync(cargoLockPath)) return;
  let cargoLock = fs.readFileSync(cargoLockPath, "utf8");
  // Update the version that belongs to the bili-shadowreplay package entry.
  cargoLock = cargoLock.replace(
    /(name = "bili-shadowreplay"\nversion = ")[^"]*(")/,
    `$1${version}$2`
  );
  fs.writeFileSync(cargoLockPath, cargoLock);
  console.log(`✅ Updated Cargo.lock version to ${version}`);
}

function main() {
  const args = process.argv.slice(2);

  // With an explicit argument: behave like the old `yarn bump <version>`.
  // Without arguments (e.g. invoked from the npm "version" hook): read the
  // version that has already been written to package.json and sync the rest.
  let version = args[0];

  if (version) {
    if (!/^\d+\.\d+\.\d+/.test(version)) {
      console.error(
        "❌ Invalid version format. Please use semantic versioning (e.g., 3.1.0)"
      );
      process.exit(1);
    }
    updatePackageJson(version);
  } else {
    version = readPackageVersion();
  }

  try {
    updateCargoToml(version);
    updateCargoLock(version);
    console.log(`🎉 Successfully synced version to ${version}`);
  } catch (error) {
    console.error("❌ Error updating version:", error.message);
    process.exit(1);
  }
}

main();
