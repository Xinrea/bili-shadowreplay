import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

export function releaseInfo(version, ref) {
  const parsed = /^(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(version);
  if (!parsed) throw new Error(`Invalid release version: ${version}`);
  const tag = `v${version}`;
  if (ref.startsWith("refs/tags/") && ref !== `refs/tags/${tag}`) {
    throw new Error(`Release tag ${ref} does not match package version ${tag}`);
  }
  return { tag, prerelease: Boolean(parsed[2]) };
}

// Match tauri-action's release asset naming, including its filename sanitizing.
function assetName(file, target) {
  const name = target.startsWith("darwin-")
    ? `${basename(file, ".app.tar.gz")}_${target === "darwin-x86_64" ? "x64" : "aarch64"}.app.tar.gz`
    : basename(file);
  return name.trim().replace(/[ ()[\]{}]/g, ".")
    .replace(/\.\./g, ".").normalize("NFD").replace(/[\u0300-\u036f]/g, "");
}

export function createManifest({ artifacts, target, version, repository, readSignature = (file) => readFileSync(file, "utf8") }) {
  const suffix = target.startsWith("darwin-") ? ".app.tar.gz.sig"
    : target.startsWith("linux-") ? ".AppImage.sig" : "-setup.exe.sig";
  const signatureFile = artifacts.find((file) => file.endsWith(suffix));
  if (!signatureFile) throw new Error(`No signed updater artifact for ${target}`);
  const bundle = signatureFile.slice(0, -4);
  if (!artifacts.includes(bundle)) throw new Error(`Missing updater bundle: ${bundle}`);
  const signature = readSignature(signatureFile).trim();
  if (!signature) throw new Error(`Empty updater signature: ${signatureFile}`);
  return {
    version,
    platforms: {
      [target]: {
        url: `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(assetName(bundle, target))}`,
        signature,
      },
    },
  };
}

export function mergeManifests(manifests) {
  const version = manifests[0]?.version;
  if (!version) throw new Error("No updater manifests provided");
  if (releaseInfo(version, "").prerelease) throw new Error("Prereleases cannot be published to the stable updater index");
  const platforms = {};
  for (const manifest of manifests) {
    if (manifest.version !== version) throw new Error("Updater versions do not match");
    for (const [target, entry] of Object.entries(manifest.platforms)) {
      if (platforms[target]) throw new Error(`Duplicate updater target: ${target}`);
      platforms[target] = entry;
    }
  }
  // A partial build must never replace the manifest for a complete release.
  for (const target of ["darwin-x86_64", "darwin-aarch64", "linux-x86_64", "windows-x86_64-cpu", "windows-x86_64-cuda"]) {
    if (!platforms[target]) throw new Error(`Missing updater target: ${target}`);
  }
  return { version, platforms };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [command, output, ...inputs] = process.argv.slice(2);
  let manifest;
  if (command === "release-info") {
    const { version } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
    const { tag, prerelease } = releaseInfo(version, process.env.GITHUB_REF);
    appendFileSync(process.env.GITHUB_OUTPUT, `tag=${tag}\nprerelease=${prerelease}\n`);
  } else if (command === "create") {
    manifest = createManifest({
      artifacts: JSON.parse(process.env.UPDATER_ARTIFACTS),
      target: process.env.UPDATER_TARGET,
      version: process.env.UPDATER_VERSION,
      repository: process.env.GITHUB_REPOSITORY,
    });
  } else if (command === "merge") {
    manifest = mergeManifests(inputs.map((file) => JSON.parse(readFileSync(file, "utf8"))));
  } else {
    throw new Error("Usage: updater-manifest.mjs release-info | create|merge OUTPUT [INPUT...]");
  }
  if (manifest) writeFileSync(output, JSON.stringify(manifest, null, 2) + "\n");
}
