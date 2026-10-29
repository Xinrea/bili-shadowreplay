import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/GenerateWholeClipModal.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const output = ts.transpileModule(`${script}
  export { generateWholeClip, closeModal, loadWholeClipArchives };
  export function configure(settings, danmu, ids = ["second", "first"]) {
    renderSettings = settings; encodeDanmu = danmu; selectedLiveIds = ids;
  }
  export function state() {
    return { showModal, isSubmitting, generationError, renderSettings };
  }
`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const plain = (value) => JSON.parse(JSON.stringify(value));
const defaults = () => ({ codec: "hevc", priority: "speed", bitrate_kbps: 10000 });

async function fixture() {
  const requests = [], generated = [];
  const context = vm.createContext({
    $state: (value) => value,
    $derived: (value) => value,
    $bindable: (value) => value,
    $effect() {},
    $props: () => ({
      showModal: true,
      archive: { title: "Archive", room_id: "room", parent_id: "parent", platform: "bilibili" },
      roomId: "room",
      onGenerated: () => generated.push(true),
    }),
    console: { error() {} },
  });
  const dependencies = {
    svelte: { untrack: (fn) => fn() },
    "../project-timeline": { defaultRenderSettings: defaults },
    "../invoker": {
      get_static_url: async () => "",
      invoke: (command, args) => {
        if (command === "get_archives_by_parent_id") return Promise.resolve([
          { live_id: "first", created_at: "2026-01-01", platform: "bilibili", room_id: "room" },
          { live_id: "second", created_at: "2026-01-02", platform: "bilibili", room_id: "room" },
        ]);
        assert.equal(command, "generate_whole_clip");
        return new Promise((resolve, reject) => requests.push({ args, resolve, reject }));
      },
    },
  };
  const module = new vm.SourceTextModule(output, { context });
  await module.link((name) => {
    const exports = dependencies[name];
    assert.ok(exports, `Unexpected dependency: ${name}`);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return { handlers: module.namespace, requests, generated };
}

test("complete recording generation sends HEVC/speed/10 Mbps defaults without danmaku", async () => {
  const f = await fixture();
  await f.handlers.loadWholeClipArchives("room", "parent");
  const completion = f.handlers.generateWholeClip();
  const request = f.requests[0];
  assert.deepEqual(plain(request.args.renderSettings), defaults());
  assert.deepEqual(plain(request.args.selectedLiveIds), ["first", "second"]);
  assert.equal(request.args.encodeDanmu, false);
  assert.equal(request.args.platform, "bilibili");
  assert.equal(request.args.roomId, "room");
  assert.equal(request.args.parentId, "parent");
  request.resolve({ id: "task" });
  await completion;
  assert.equal(f.handlers.state().showModal, false);
  assert.deepEqual(f.generated, [true]);
});

for (const danmu of [false, true]) {
  test(`complete recording snapshots all settings and selection with danmaku=${danmu}`, async () => {
    const f = await fixture();
    const settings = { codec: "h264", priority: "quality", bitrate_kbps: 16000 };
    const ids = ["second", "first"];
    f.handlers.configure(settings, danmu, ids);
    const completion = f.handlers.generateWholeClip();
    settings.codec = "hevc";
    settings.bitrate_kbps = 6000;
    ids.reverse();
    const request = f.requests[0];
    assert.deepEqual(plain(request.args.renderSettings), {
      codec: "h264", priority: "quality", bitrate_kbps: 16000,
    });
    assert.deepEqual(plain(request.args.selectedLiveIds), ["second", "first"]);
    assert.equal(request.args.encodeDanmu, danmu);
    assert.equal(request.args.outputName, undefined);
    // The queue submission may be slow; don't create duplicate whole recordings.
    await f.handlers.generateWholeClip();
    f.handlers.closeModal();
    assert.equal(f.requests.length, 1);
    assert.equal(f.handlers.state().showModal, true);
    request.resolve({ id: "task" });
    await completion;
    assert.equal(f.handlers.state().isSubmitting, false);
  });
}

test("submission errors stay visible and allow retry without losing encoding settings", async () => {
  const f = await fixture();
  const settings = { codec: "h264", priority: "quality", bitrate_kbps: 20000 };
  f.handlers.configure(settings, false);
  const first = f.handlers.generateWholeClip();
  f.requests[0].reject("queue full");
  await first;
  assert.match(f.handlers.state().generationError, /queue full/);
  assert.equal(f.handlers.state().showModal, true);
  assert.equal(f.handlers.state().isSubmitting, false);
  assert.deepEqual(f.generated, []);
  const retry = f.handlers.generateWholeClip();
  assert.equal(f.handlers.state().generationError, "");
  assert.deepEqual(plain(f.requests[1].args.renderSettings), settings);
  f.requests[1].resolve({ id: "task" });
  await retry;
  assert.deepEqual(f.generated, [true]);
});

test("empty selection cannot generate a complete recording", async () => {
  const f = await fixture();
  await f.handlers.generateWholeClip();
  assert.deepEqual(f.requests, []);
});
