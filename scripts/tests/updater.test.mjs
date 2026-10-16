import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { createManifest, mergeManifests } from "../updater-manifest.mjs";

async function fixture({ desktop = true, check = async () => null, relaunch = async () => {} } = {}) {
  const context = vm.createContext({ __APP_VERSION__: "2.24.0" });
  const writable = (initial) => {
    let value = initial;
    return { set(next) { value = next; }, get value() { return value; } };
  };
  const dependencies = {
    "svelte/store": { writable },
    "@tauri-apps/plugin-updater": { check },
    "@tauri-apps/plugin-process": { relaunch },
    "../invoker": { TAURI_ENV: desktop },
  };
  const source = await readFile(new URL("../../src/lib/stores/version.ts", import.meta.url), "utf8");
  const output = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } }).outputText;
  const module = new vm.SourceTextModule(output, { context });
  await module.link((name) => {
    const exports = dependencies[name];
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return module.namespace;
}

test("stable version detection never offers a downgrade or prerelease", async () => {
  const { isNewerVersion: newer } = await fixture();
  assert.equal(newer("v2.24.0", "2.24.0"), false);
  assert.equal(newer("v2.23.9", "2.24.0"), false);
  assert.equal(newer("v2.25.0-rc.1", "2.24.0"), false);
  assert.equal(newer("v2.24.0", "2.24.0-rc.1"), true);
  assert.equal(newer("v2.100.0", "2.24.0"), true);
  assert.equal(newer("garbage", "2.24.0"), false);
});

test("download progress is retained, installation completes before restart, and repeated clicks are ignored", async () => {
  let finish, downloads = 0, restarts = 0;
  const f = await fixture({
    check: async () => ({ version: "2.25.0", downloadAndInstall: async (progress) => {
      downloads++;
      progress({ event: "Started", data: { contentLength: 200 } });
      progress({ event: "Progress", data: { chunkLength: 100 } });
      await new Promise((resolve) => { finish = resolve; });
      progress({ event: "Finished" });
    } }),
    relaunch: async () => { restarts++; },
  });
  await f.checkForUpdates();
  assert.equal(f.hasNewVersion.value, true);
  const installing = f.installUpdate();
  assert.equal(f.updateStatus.value.phase, "downloading");
  assert.equal(f.updateStatus.value.downloaded, 100);
  assert.equal(f.updateStatus.value.total, 200);
  await f.installUpdate();
  await f.checkForUpdates();
  assert.equal(downloads, 1);
  assert.equal(restarts, 0);
  finish(); await installing;
  assert.equal(restarts, 1);
  assert.equal(f.updateStatus.value.phase, "restarting");
});

test("failed download or signature verification never restarts and can be retried", async () => {
  let downloads = 0, restarts = 0;
  const f = await fixture({
    check: async () => ({ version: "2.25.0", downloadAndInstall: async () => {
      if (++downloads === 1) throw new Error("invalid signature");
    } }),
    relaunch: async () => { restarts++; },
  });
  await f.installUpdate();
  assert.equal(f.updateStatus.value.phase, "error");
  assert.match(f.updateStatus.value.error, /invalid signature/);
  assert.equal(restarts, 0);
  await f.installUpdate();
  assert.equal(downloads, 2);
  assert.equal(restarts, 1);
});

test("restart retry does not install or download a second time", async () => {
  let downloads = 0, restarts = 0;
  const f = await fixture({
    check: async () => ({ version: "2.25.0", downloadAndInstall: async () => { downloads++; } }),
    relaunch: async () => { if (++restarts === 1) throw new Error("restart failed"); },
  });
  await f.installUpdate();
  assert.equal(f.updateStatus.value.phase, "restart-required");
  await f.checkForUpdates();
  assert.equal(f.updateStatus.value.phase, "restart-required");
  await f.installUpdate();
  assert.equal(downloads, 1);
  assert.equal(restarts, 2);
});

test("checks share a request, close old resources, clear stale notifications, and report network errors", async () => {
  let finish, checks = 0, closed = 0;
  const f = await fixture({ check: () => {
    checks++;
    return new Promise((resolve, reject) => { finish = { resolve, reject }; });
  } });
  const first = f.checkForUpdates();
  const duplicate = f.checkForUpdates();
  assert.equal(checks, 1);
  finish.resolve({ version: "2.25.0", close: async () => { closed++; } });
  await Promise.all([first, duplicate]);
  assert.equal(f.latestVersion.value, "v2.25.0");
  const second = f.checkForUpdates(); finish.resolve(null); await second;
  assert.equal(closed, 1);
  assert.equal(f.hasNewVersion.value, false);
  const third = f.checkForUpdates(); finish.reject(new Error("offline")); await third;
  assert.equal(f.updateStatus.value.phase, "error");
});

test("web deployments never call desktop update or restart APIs", async () => {
  const unexpected = () => { throw new Error("desktop API called in web mode"); };
  const f = await fixture({ desktop: false, check: unexpected, relaunch: unexpected });
  await f.checkForUpdates(); await f.installUpdate();
  assert.equal(f.updateStatus.value.phase, "idle");
});

const targets = ["darwin-x86_64", "darwin-aarch64", "linux-x86_64", "windows-x86_64-cpu", "windows-x86_64-cuda"];
function manifest(target) {
  const name = target.startsWith("darwin") ? "bili-shadowreplay.app.tar.gz"
    : target.startsWith("linux") ? "bili-shadowreplay_2.25.0_amd64.AppImage"
    : `bili-shadowreplay-${target.endsWith("cuda") ? "cuda" : "cpu"}_2.25.0_x64-setup.exe`;
  return createManifest({ artifacts: [`/bundle/${name}`, `/bundle/${name}.sig`],
    target, version: "2.25.0", repository: "Xinrea/bili-shadowreplay", readSignature: () => "signed\n" });
}

test("release manifests preserve macOS architecture suffixes and isolate CPU/CUDA installers", () => {
  const result = mergeManifests(targets.map(manifest));
  assert.match(result.platforms["darwin-x86_64"].url, /bili-shadowreplay_x64.app.tar.gz$/);
  assert.match(result.platforms["darwin-aarch64"].url, /bili-shadowreplay_aarch64.app.tar.gz$/);
  assert.match(result.platforms["windows-x86_64-cpu"].url, /-cpu_/);
  assert.match(result.platforms["windows-x86_64-cuda"].url, /-cuda_/);
  assert.equal(result.platforms["linux-x86_64"].signature, "signed");
});

test("incomplete, duplicate, mismatched or unsigned release manifests fail closed", () => {
  const all = targets.map(manifest);
  assert.throws(() => mergeManifests(all.slice(1)), /Missing updater target/);
  assert.throws(() => mergeManifests([...all, all[0]]), /Duplicate/);
  assert.throws(() => mergeManifests([all[0], { ...all[1], version: "2.26.0" }]), /versions do not match/);
  assert.throws(() => createManifest({ artifacts: [], target: "darwin-aarch64" }), /No signed updater/);
});
