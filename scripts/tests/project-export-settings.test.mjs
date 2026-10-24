import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/project/ProjectEditor.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("ProjectEditor.ts", script, ts.ScriptTarget.Latest, true);
const names = ["openExportSettings", "confirmExport", "exportProject"];
const handlers = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && names.includes(node.name?.text));
assert.equal(handlers.length, names.length);
// Exercise the actual async export handlers without mounting the unrelated
// player/editor UI. Only their I/O and surrounding component state are mocked.
const code = ts.transpileModule(handlers.map((node) => node.getText(parsed)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const plain = (value) => JSON.parse(JSON.stringify(value));
const defaults = () => ({ quality: "balanced", bitrate_kbps: 0 });

function fixture({ desktop = true, pick = async () => "/export.mp4", danmu = false } = {}) {
  const saved = { clips: [{ id: "clip", material_id: 1, start: 0, end: 10 }], danmu, render_settings: defaults() };
  const patches = [], exports = [], calls = [], alerts = [];
  const state = {
    preparingExport: false,
    exporting: null,
    project: { id: 1 },
    projectId: 1,
    title: "Test clip",
    player: null,
    timeline: plain(saved),
    syncedTimeline: JSON.stringify(saved),
    showExportSettings: false,
    exportDanmu: false,
    exportSettings: defaults(),
    exportBase: null,
    defaultRenderSettings: defaults,
    $state: { snapshot: plain },
    TAURI_ENV: desktop,
    save: async () => { calls.push("pick"); return pick(); },
    saveAll: async () => { calls.push("saveTimeline"); },
    projectRefresh: { run: async () => {
      calls.push("refresh");
      state.timeline = plain(saved);
      state.syncedTimeline = JSON.stringify(saved);
    } },
    invoke: async (command, args) => {
      calls.push(command);
      if (command === "update_project_output_settings") {
        patches.push(plain(args));
        if (args.danmu !== null) saved.danmu = args.danmu;
        if (args.renderSettings !== null) saved.render_settings = plain(args.renderSettings);
      } else if (command === "export_project") {
        exports.push(plain(args));
        return { path: args.target };
      } else {
        throw new Error(`Unexpected command: ${command}`);
      }
    },
    finishEditing() {},
    settleOverlayEdit() {},
    renderOverlayImages: async () => [],
    stickerRefUrl() {},
    visibleSubtitles: () => [],
    frameRate: 30,
    projectCues: () => [],
    subtitleStyle: {},
    buildSubtitleAss: () => "",
    generateEventId: () => "export-1",
    listen: async () => () => {},
    announceTask() {},
    openTaskCenter() {},
    settleTask() {},
    parseTimeline: JSON.parse,
    alert: (message) => alerts.push(message),
  };
  const context = vm.createContext(state);
  vm.runInContext(code, context);
  return { state, saved, patches, exports, calls, alerts };
}

test("an untouched export dialog uses settings saved by another window", async () => {
  const f = fixture();
  await f.state.openExportSettings();
  f.saved.danmu = true;
  f.saved.render_settings = { quality: "balanced", bitrate_kbps: 16000 };
  // A background refresh must not redefine which options the user edited.
  await f.state.projectRefresh.run();
  await f.state.confirmExport();
  assert.deepEqual(f.patches, []);
  assert.deepEqual(f.exports[0].timeline, f.saved);
  assert.equal(f.state.showExportSettings, false);
  assert.deepEqual(f.alerts, []);
});

test("editing only danmaku preserves encoding changed while the file picker is open", async () => {
  const f = fixture({ pick: async () => {
    assert.deepEqual(f.patches, []);
    f.saved.render_settings = { quality: "balanced", bitrate_kbps: 16000 };
    return "/export.mp4";
  } });
  await f.state.openExportSettings();
  f.state.exportDanmu = true;
  await f.state.confirmExport();
  assert.deepEqual(f.patches, [{ id: 1, danmu: true, renderSettings: null }]);
  assert.equal(f.exports[0].timeline.render_settings.bitrate_kbps, 16000);
  assert.equal(f.exports[0].timeline.danmu, true);
  assert.ok(f.calls.indexOf("pick") < f.calls.indexOf("update_project_output_settings"));
  assert.deepEqual(f.alerts, []);
});

test("editing encoding preserves remote danmaku and writes the complete encoding mode", async () => {
  const f = fixture();
  f.saved.render_settings = { quality: "balanced", bitrate_kbps: 8000 };
  await f.state.openExportSettings();
  f.saved.danmu = true;
  f.state.exportSettings = { quality: "high", bitrate_kbps: 0 };
  await f.state.confirmExport();
  assert.deepEqual(f.patches, [{ id: 1, danmu: null, renderSettings: { quality: "high", bitrate_kbps: 0 } }]);
  assert.equal(f.exports[0].timeline.danmu, true);
  assert.deepEqual(f.exports[0].timeline.render_settings, f.state.exportSettings);
  assert.deepEqual(f.alerts, []);
});

test("cancelling the file picker persists nothing and keeps the export dialog editable", async () => {
  const f = fixture({ pick: async () => null });
  await f.state.openExportSettings();
  const before = plain(f.saved);
  f.state.exportDanmu = true;
  f.state.exportSettings = { quality: "balanced", bitrate_kbps: 16000 };
  await f.state.confirmExport();
  assert.deepEqual(f.patches, []);
  assert.deepEqual(f.exports, []);
  assert.deepEqual(f.saved, before);
  assert.equal(f.calls.includes("saveTimeline"), false);
  assert.equal(f.state.showExportSettings, true);
  assert.equal(f.state.preparingExport, false);
  assert.deepEqual(f.alerts, []);
});

test("options restored to their opening values do not overwrite remote updates", async () => {
  const f = fixture();
  await f.state.openExportSettings();
  f.state.exportDanmu = true;
  f.state.exportSettings = { quality: "high", bitrate_kbps: 0 };
  f.state.exportDanmu = false;
  f.state.exportSettings = defaults();
  f.saved.danmu = true;
  f.saved.render_settings = { quality: "compact", bitrate_kbps: 0 };
  await f.state.confirmExport();
  assert.deepEqual(f.patches, []);
  assert.deepEqual(f.exports[0].timeline, f.saved);
});

test("web export saves explicit false and new settings without opening a file picker", async () => {
  const f = fixture({ desktop: false, danmu: true });
  await f.state.openExportSettings();
  f.state.exportDanmu = false;
  f.state.exportSettings = { quality: "balanced", bitrate_kbps: 12000 };
  await f.state.confirmExport();
  assert.deepEqual(f.patches, [{ id: 1, danmu: false, renderSettings: { quality: "balanced", bitrate_kbps: 12000 } }]);
  assert.equal(f.calls.includes("pick"), false);
  assert.equal(f.exports[0].target, null);
  assert.equal(f.exports[0].timeline.danmu, false);
  assert.deepEqual(f.alerts, []);
});
