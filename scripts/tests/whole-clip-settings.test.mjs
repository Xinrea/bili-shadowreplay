import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/GenerateWholeClipModal.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const output = ts.transpileModule(`${script}
  export { generateWholeClip, closeModal, loadWholeClipArchives, handleKeydown };
  export function configure(settings, danmu, ids = ["second", "first"], encode = true) {
    renderSettings = settings; encodeDanmu = danmu; selectedLiveIds = ids; reencode = encode;
  }
  export function state() {
    return { showModal, isSubmitting, generationError, renderSettings };
  }
`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const plain = (value) => JSON.parse(JSON.stringify(value));
const defaults = () => ({ codec: "hevc", priority: "speed", bitrate_kbps: 10000 });

async function fixture() {
  const requests = [], generated = [];
  let visible = true;
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
    "../page-visibility": { pageVisibility: () => () => visible },
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
  return { handlers: module.namespace, requests, generated, setVisible: value => { visible = value; } };
}

test("complete recording generation defaults to stream copy without danmaku", async () => {
  const f = await fixture();
  await f.handlers.loadWholeClipArchives("room", "parent");
  const completion = f.handlers.generateWholeClip();
  const request = f.requests[0];
  assert.equal(request.args.renderSettings, undefined);
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

test("Escape is consumed by the nested recording dialog and cannot dismiss its parent", async () => {
  const f = await fixture();
  let prevented = 0;
  let stopped = 0;
  const event = { key: "Escape", isComposing: false, preventDefault() { prevented++; }, stopPropagation() { stopped++; } };
  f.handlers.handleKeydown({ ...event, isComposing: true });
  assert.equal(f.handlers.state().showModal, true);
  f.handlers.configure(defaults(), false, ["first"]);
  const pending = f.handlers.generateWholeClip();
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.state().showModal, true, "submitting keeps the child dialog open");
  assert.equal(prevented, 1);
  assert.equal(stopped, 1);
  f.requests[0].reject(new Error("retry"));
  await pending;
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.state().showModal, false);
  assert.equal(stopped, 2);
});

test("complete recording dialog ignores inactive-page and already handled Escape", async () => {
  const f = await fixture();
  let prevented = 0, stopped = 0;
  const event = { key: "Escape", isComposing: false, preventDefault() { prevented++; }, stopPropagation() { stopped++; } };
  f.setVisible(false);
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.state().showModal, true);
  assert.equal(prevented, 0);
  assert.equal(stopped, 0);
  f.setVisible(true);
  f.handlers.handleKeydown({ ...event, defaultPrevented: true });
  assert.equal(f.handlers.state().showModal, true);
  assert.equal(prevented, 0);
  assert.equal(stopped, 0);
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.state().showModal, false);
  assert.equal(prevented, 1);
  assert.equal(stopped, 1);
});

test("complete recording encoding is optional unless danmaku is embedded", async () => {
  const f = await fixture();
  for (const encode of [false, true]) {
    for (const danmu of [false, true, false]) {
      f.handlers.configure(defaults(), danmu, ["first", "second"], encode);
      const completion = f.handlers.generateWholeClip();
      const request = f.requests.at(-1);
      assert.equal(request.args.encodeDanmu, danmu);
      assert.deepEqual(
        plain(request.args).renderSettings,
        danmu || encode ? defaults() : undefined,
      );
      request.resolve({ id: "task" });
      await completion;
    }
  }
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
