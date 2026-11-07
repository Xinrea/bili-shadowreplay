import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function handlers(path, names) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile(path, script, ts.ScriptTarget.Latest, true);
  const functions = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && names.includes(node.name?.text));
  assert.equal(functions.length, names.length);
  return ts.transpileModule(functions.map((node) => node.getText(parsed)).join("\n"), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}

// Exercise the real handlers; only the surrounding UI state and I/O are mocked.
const editorCode = await handlers("../../src/lib/components/project/ProjectEditor.svelte", ["addMaterialAtPlayhead", "addMaterialAtTime"]);
const timelineCode = await handlers("../../src/lib/components/project/ProjectTimeline.svelte", ["dragOver", "drop"]);
const plain = (value) => JSON.parse(JSON.stringify(value));

function editor() {
  const additions = [], overlays = [];
  const state = {
    time: 1,
    mainLength: 20,
    timeline: { clips: [{ id: "base", start: 0, end: 20 }] },
    MIN_OVERLAY_DURATION: 0.1,
    loadMaterial: async () => ({ duration: 8, url: "material.mp4" }),
    probeAspect: async () => 16 / 9,
    filledElement: (aspect) => ({ aspect }),
    addMaterial: async (id) => additions.push(id),
    insertOverlay: (...args) => overlays.push(plain(args)),
  };
  vm.runInContext(editorCode, vm.createContext(state));
  return { state, additions, overlays };
}

test("dropping on existing picture overlays at pointer time, not the playhead, without cutting the main clip", async () => {
  const { state, additions, overlays } = editor();
  const before = plain(state.timeline);
  await state.addMaterialAtTime(42, 7.25);
  assert.deepEqual(additions, []);
  assert.equal(overlays.length, 1);
  assert.equal(overlays[0][0].material_id, 42);
  assert.deepEqual(overlays[0][0].element, { aspect: 16 / 9 });
  assert.equal(overlays[0][1], 7.25);
  assert.equal(overlays[0][2], 8);
  assert.deepEqual(plain(state.timeline), before);
});

test("drops at the main end and on an empty project append instead of creating an unanchored overlay", async () => {
  const { state, additions, overlays } = editor();
  await state.addMaterialAtTime(42, 20);
  state.timeline = { clips: [] };
  state.mainLength = 0;
  await state.addMaterialAtTime(43, 0);
  assert.deepEqual(additions, [42, 43]);
  assert.deepEqual(overlays, []);
});

test("the plus button still uses the playhead and treats its last minimum span as append", async () => {
  const { state, additions, overlays } = editor();
  state.time = 4;
  await state.addMaterialAtPlayhead(42);
  assert.equal(overlays[0][1], 4);
  state.time = 19.95;
  await state.addMaterialAtPlayhead(43);
  assert.deepEqual(additions, [43]);
});

test("a pending material drop uses the current main track after asynchronous loading", async () => {
  const { state, additions, overlays } = editor();
  state.probeAspect = async () => {
    state.timeline = { clips: [] };
    state.mainLength = 0;
    return 16 / 9;
  };
  await state.addMaterialAtTime(42, 7);
  assert.deepEqual(additions, [42]);
  assert.deepEqual(overlays, []);
});

function dropFixture() {
  const drops = [];
  const state = {
    MATERIAL_DRAG_TYPE: "application/x-bsr-material",
    mainLength: 20,
    latestStart: 19.9,
    time: 1,
    materialDropTime: null,
    snapTime: null,
    timeAt: (x) => x,
    clipEdges: () => [0, 20],
    snap: (value) => value,
    onDropMaterial: (...args) => drops.push(args),
  };
  vm.runInContext(timelineCode, vm.createContext(state));
  const event = (x, id = "42") => ({
    clientX: x,
    shiftKey: false,
    dataTransfer: { types: [state.MATERIAL_DRAG_TYPE], getData: () => id },
    preventDefault() {},
  });
  return { state, drops, event };
}

test("timeline drag-over previews the pointer time and the drop forwards it unchanged", () => {
  const { state, drops, event } = dropFixture();
  state.dragOver(event(7.25));
  assert.equal(state.materialDropTime, 7.25);
  state.drop(event(7.25));
  assert.deepEqual(drops, [[42, 7.25]]);
  assert.equal(state.materialDropTime, null);
  assert.equal(state.snapTime, null);
});

test("drop markers clamp to the append point and malformed materials are ignored", () => {
  const { state, drops, event } = dropFixture();
  state.dragOver(event(25));
  assert.equal(state.materialDropTime, 20);
  state.drop(event(25));
  assert.deepEqual(drops, [[42, 20]]);
  for (const id of ["", "invalid", "-1", "Infinity"]) {
    state.drop(event(4, id));
  }
  assert.equal(drops.length, 1);
});
