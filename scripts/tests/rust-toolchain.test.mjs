import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = path => readFile(new URL(path, root), "utf8");
const toolchain = await read("rust-toolchain.toml");
const channel = toolchain.match(/^channel = "(\d+\.\d+\.\d+)"$/m)?.[1];
assert.ok(channel, "Pin an exact Rust release, not a moving stable channel");

test("CI Rust installations match the repository toolchain", async () => {
  let installations = 0;
  for (const file of await readdir(new URL(".github/workflows/", root))) {
    if (!/\.ya?ml$/.test(file)) continue;
    const workflow = await read(`.github/workflows/${file}`);
    for (const match of workflow.matchAll(/uses:\s*dtolnay\/rust-toolchain@([^\s#]+)/g)) {
      assert.equal(match[1], channel, `${file} must use Rust ${channel}`);
      installations++;
    }
  }
  assert.ok(installations > 0);
  assert.match(await read(".github/workflows/lint.yml"), /- 'rust-toolchain\.toml'/);
});

test("container builds match the repository toolchain", async () => {
  const docker = await read("Dockerfile");
  assert.equal(docker.match(/^FROM rust:([\d.]+)-slim-trixie AS rust-base$/m)?.[1], channel);
  assert.match(docker, /^COPY rust-toolchain\.toml \/app\/rust-toolchain\.toml$/m);
  const devcontainer = await read(".devcontainer/Dockerfile");
  assert.equal(devcontainer.match(/--default-toolchain ([\d.]+)/)?.[1], channel);
});

test("all workspace packages inherit the shared minimum Rust version", async () => {
  const manifest = await read("src-tauri/Cargo.toml");
  const baseline = manifest.match(/\[workspace\.package\]\s+rust-version = "([^"]+)"/)?.[1];
  assert.equal(baseline, channel.split(".").slice(0, 2).join("."));
  assert.match(manifest, /^rust-version\.workspace = true$/m);
  const members = manifest.match(/^members = \[(.+)\]$/m)?.[1].matchAll(/"([^"]+)"/g);
  assert.ok(members);
  for (const [, member] of members) {
    assert.match(await read(`src-tauri/${member}/Cargo.toml`), /^rust-version\.workspace = true$/m, member);
  }
});
