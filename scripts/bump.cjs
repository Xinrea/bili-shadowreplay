#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

function updatePackageJson(version) {
  const packageJsonPath = path.join(process.cwd(), "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  packageJson.version = version;
  fs.writeFileSync(
    packageJsonPath,
    JSON.stringify(packageJson, null, 2) + "\n"
  );
  console.log(`✅ Updated package.json version to ${version}`);
}

function updateCargoToml(version) {
  const cargoTomlPath = path.join(process.cwd(), "src-tauri", "Cargo.toml");
  let cargoToml = fs.readFileSync(cargoTomlPath, "utf8");

  // Update the version in the [package] section
  cargoToml = cargoToml.replace(/^version = ".*"$/m, `version = "${version}"`);

  fs.writeFileSync(cargoTomlPath, cargoToml);
  console.log(`✅ Updated Cargo.toml version to ${version}`);
}

function main() {
  const args = process.argv.slice(2);

  if (args.length === 0) {
    console.error("❌ Please provide a version number");
    console.error("Usage: yarn bump <version>");
    console.error("Example: yarn bump 3.1.0");
    process.exit(1);
  }

  const version = args[0];

  // Validate version format (simple check)
  if (!/^\d+\.\d+\.\d+/.test(version)) {
    console.error(
      "❌ Invalid version format. Please use semantic versioning (e.g., 3.1.0)"
    );
    process.exit(1);
  }

  try {
    updatePackageJson(version);
    updateCargoToml(version);
    console.log(`🎉 Successfully bumped version to ${version}`);
  } catch (error) {
    console.error("❌ Error updating version:", error.message);
    process.exit(1);
  }
}

main();
