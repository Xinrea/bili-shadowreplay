import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/page/Setting.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("Setting.ts", script, ts.ScriptTarget.Latest, true);
const names = ["get_config", "updateAutoGenerate"];
const handlers = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && names.includes(node.name?.text));
assert.equal(handlers.length, names.length);
const code = ts.transpileModule(handlers.map((node) => node.getText(parsed)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const timelineSource = await readFile(new URL("../../src/lib/project-timeline.ts", import.meta.url), "utf8");
const timeline = new vm.SourceTextModule(ts.transpileModule(timelineSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText);
await timeline.link(() => { throw new Error("Unexpected timeline dependency"); });
await timeline.evaluate();
const plain = (value) => JSON.parse(JSON.stringify(value));
const defaults = () => plain(timeline.namespace.defaultRenderSettings());

function fixture(autoGenerate = { enabled: false, encode_danmu: false }) {
  const saved = { auto_generate: plain(autoGenerate) };
  const requests = [];
  const state = {
    setting_model: { auto_generate: { enabled: false, encode_danmu: false, render_settings: defaults() } },
    autoGenerateLoading: true,
    autoGenerateSaving: false,
    autoGenerateError: "",
    normalizeRenderSettings: timeline.namespace.normalizeRenderSettings,
    console: { log() {} },
    invoke: (command, args) => {
      if (command === "get_config") return Promise.resolve(plain(saved));
      assert.equal(command, "update_auto_generate");
      return new Promise((resolve, reject) => requests.push({
        args,
        reject,
        resolve() {
          saved.auto_generate = {
            enabled: args.enabled,
            encode_danmu: args.encodeDanmu,
            render_settings: plain(args.renderSettings),
          };
          resolve();
        },
      }));
    },
  };
  vm.runInContext(code, vm.createContext(state));
  return { state, saved, requests };
}

test("old auto-generation config gains HEVC/speed/10 Mbps without enabling automation", async () => {
  const f = fixture();
  await f.state.get_config();
  assert.equal(f.state.autoGenerateLoading, false);
  assert.deepEqual(plain(f.state.setting_model.auto_generate), {
    enabled: false, encode_danmu: false, render_settings: defaults(),
  });
  assert.deepEqual(f.requests, []);
});

for (const [field, value] of [["codec", "h264"], ["priority", "quality"], ["bitrate_kbps", 16000]]) {
  test(`editing automatic ${field} saves the complete independent encoding choice`, async () => {
    const f = fixture({ enabled: true, encode_danmu: false, render_settings: defaults() });
    await f.state.get_config();
    f.state.setting_model.auto_generate.render_settings[field] = value;
    const completion = f.state.updateAutoGenerate();
    assert.deepEqual(plain(f.requests[0].args), {
      enabled: true,
      encodeDanmu: false,
      renderSettings: { ...defaults(), [field]: value },
    });
    f.requests[0].resolve();
    await completion;
    await f.state.get_config();
    assert.deepEqual(plain(f.state.setting_model.auto_generate.render_settings), { ...defaults(), [field]: value });
  });
}

test("disabling, re-enabling and changing danmaku preserve saved encoding settings", async () => {
  const settings = { codec: "h264", priority: "quality", bitrate_kbps: 20000 };
  const f = fixture({ enabled: true, encode_danmu: false, render_settings: settings });
  await f.state.get_config();
  for (const [enabled, danmu] of [[false, false], [true, false], [true, true]]) {
    f.state.setting_model.auto_generate.enabled = enabled;
    f.state.setting_model.auto_generate.encode_danmu = danmu;
    const completion = f.state.updateAutoGenerate();
    assert.deepEqual(plain(f.requests.at(-1).args), {
      enabled, encodeDanmu: danmu, renderSettings: settings,
    });
    f.requests.at(-1).resolve();
    await completion;
  }
});

test("pending saves use a settings snapshot and do not issue concurrent writes", async () => {
  const f = fixture({ enabled: true, encode_danmu: true, render_settings: defaults() });
  await f.state.get_config();
  const completion = f.state.updateAutoGenerate();
  assert.equal(f.state.autoGenerateSaving, true);
  f.state.setting_model.auto_generate.render_settings.codec = "h264";
  await f.state.updateAutoGenerate();
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].args.renderSettings.codec, "hevc");
  f.requests[0].resolve();
  await completion;
  assert.equal(f.state.autoGenerateSaving, false);
});

test("failed saves display an error and can retry the retained choices", async () => {
  const f = fixture({ enabled: true, encode_danmu: false, render_settings: defaults() });
  await f.state.get_config();
  f.state.setting_model.auto_generate.render_settings.bitrate_kbps = 16000;
  const failed = f.state.updateAutoGenerate();
  f.requests[0].reject(new Error("disk unavailable"));
  await failed;
  assert.match(f.state.autoGenerateError, /disk unavailable/);
  assert.equal(f.state.autoGenerateSaving, false);
  assert.equal(f.saved.auto_generate.render_settings.bitrate_kbps, 10000);
  const retry = f.state.updateAutoGenerate();
  assert.equal(f.state.autoGenerateError, "");
  assert.equal(f.requests[1].args.renderSettings.bitrate_kbps, 16000);
  f.requests[1].resolve();
  await retry;
  assert.equal(f.saved.auto_generate.render_settings.bitrate_kbps, 16000);
});

test("auto-generation controls cannot write defaults before config finishes loading", async () => {
  const f = fixture();
  await f.state.updateAutoGenerate();
  assert.deepEqual(f.requests, []);
});
