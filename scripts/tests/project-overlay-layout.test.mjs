import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const transpile = (text) => ts.transpileModule(text, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const plain = (value) => JSON.parse(JSON.stringify(value));

async function fixture() {
  const transforms = [], canvases = [];
  const context = vm.createContext({
    document: { createElement() {
      const canvas = { width: 0, height: 0, getContext: () => ({ translate() {} }), toDataURL: () => "data:image/png;base64,mock" };
      canvases.push(canvas);
      return canvas;
    } },
  });
  const model = new vm.SourceTextModule(transpile(await readFile(new URL("../../src/lib/cover-elements.ts", import.meta.url), "utf8")), { context });
  const render = new vm.SyntheticModule(["drawElements", "drawSticker", "drawText", "loadSticker", "measure"], function() {
    this.setExport("drawElements", () => []);
    this.setExport("drawSticker", () => {});
    this.setExport("drawText", () => {});
    this.setExport("loadSticker", async () => ({}));
    this.setExport("measure", (text) => text.length * 20);
  }, { context });
  const timeline = new vm.SyntheticModule(["placeOverlays"], function() {
    this.setExport("placeOverlays", (timeline) => timeline.items.map((item) => ({ item })));
  }, { context });
  const module = new vm.SourceTextModule(transpile(await readFile(new URL("../../src/lib/components/project/overlay-render.ts", import.meta.url), "utf8")), { context });
  await module.link((name) => name.endsWith("cover-elements") ? model : name.endsWith("project-timeline") ? timeline : render);
  await module.evaluate();
  return {
    api: module.namespace, canvases, transforms,
    canvas(width, height) { return { canvas: { width, height }, clearRect() {}, setTransform: (...args) => transforms.push(args) }; },
  };
}
const sticker = (x = 960, y = 540) => ({ id: "sticker", kind: "sticker", element: { id: 1, kind: "sticker", x, y, size: 200, aspect: 1, rotation: 0, src: "test", flip: false } });

test("portrait overlay preview maps centers independently while keeping artwork square", async () => {
  const f = await fixture();
  f.api.drawOverlays(f.canvas(1080, 1920), [sticker()], () => "image");
  const [sx, , , sy, x, y] = f.transforms.at(-1);
  assert.equal(sx, sy);
  assert.equal(sx, 1080 / 1920);
  assert.equal(960 * sx + x, 540);
  assert.equal(540 * sy + y, 960);
});

test("wide canvas keeps overlay shape and position with height-limited scaling", async () => {
  const f = await fixture();
  f.api.drawOverlays(f.canvas(2560, 1080), [sticker(480, 270)], () => "image");
  const [sx, , , sy, x, y] = f.transforms.at(-1);
  assert.equal(sx, 1);
  assert.equal(sy, 1);
  assert.equal(480 * sx + x, 640);
  assert.equal(270 * sy + y, 270);
});

test("offscreen export PNG retains its element center before final-frame clipping", async () => {
  const f = await fixture();
  const [image] = await f.api.renderOverlayImages({ items: [sticker(4.5, 1078.5)] }, async () => "image");
  const canvas = f.canvases[0];
  assert.equal(image.x + canvas.width / 2, 4.5);
  assert.equal(image.y + canvas.height / 2, 1078.5);
  assert.ok(image.x < 0);
  assert.ok(image.y + canvas.height > 1080);
});

test("oversized overlay export fails before allocating a canvas", async () => {
  const f = await fixture();
  const item = sticker();
  item.element.size = 5000;
  await assert.rejects(f.api.renderOverlayImages({ items: [item] }, async () => "image"), /4096/);
  assert.deepEqual(plain(f.canvases), []);
});

async function stageFixture(mode = "move") {
  const f = await fixture();
  const source = await readFile(new URL("../../src/lib/components/project/OverlayStage.svelte", import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const ast = ts.createSourceFile("Stage.ts", script, ts.ScriptTarget.Latest, true);
  const names = ["boxOf", "toFrame", "hitTest", "handlePointerMove"];
  const functions = ast.statements.filter((node) => ts.isFunctionDeclaration(node) && names.includes(node.name?.text));
  const model = new vm.SourceTextModule(transpile(await readFile(new URL("../../src/lib/cover-elements.ts", import.meta.url), "utf8")));
  await model.link(() => { throw new Error("unexpected dependency"); });
  await model.evaluate();
  const item = sticker();
  const changes = [];
  const state = {
    ...model.namespace,
    unitX: 1080 / 1920, unitY: 1920 / 1080, unit: 1080 / 1920, width: 1080, height: 1920,
    surface: { getBoundingClientRect: () => ({ left: 0, top: 0 }), style: {} },
    measure: (text) => text.length * 20,
    aspectOf: () => 1,
    active: [{ item }],
    drag: { mode, id: item.id, start: { x: 640, y: 960 }, origin: item.element, kind: "sticker", moved: false },
    onChange: (id, change) => changes.push(plain(change)),
    guides: { x: false, y: false },
  };
  vm.runInContext(transpile(functions.map((node) => node.getText(ast)).join("\n")), vm.createContext(state));
  return { state, changes, f };
}

test("portrait hit testing follows the undistorted artwork bounds", async () => {
  const { state } = await stageFixture();
  assert.equal(state.hitTest({ clientX: 590, clientY: 960 }).item.id, "sticker");
  assert.equal(state.hitTest({ clientX: 540, clientY: 1025 }), null);
});

test("portrait overlay rotation and resize follow screen geometry", async () => {
  const rotating = await stageFixture("rotate");
  rotating.state.handlePointerMove({ clientX: 540, clientY: 1060, shiftKey: false });
  assert.equal(rotating.changes[0].rotation, 90);
  const resizing = await stageFixture("scale");
  resizing.state.handlePointerMove({ clientX: 740, clientY: 960, shiftKey: false });
  assert.equal(resizing.changes[0].size, 400);
});

test("portrait overlay drag maps pointer displacement back to normalized positions", async () => {
  const { state, changes } = await stageFixture();
  state.handlePointerMove({ clientX: 740, clientY: 1060, shiftKey: false });
  assert.deepEqual(changes[0], { x: Math.round(960 + 100 / state.unitX), y: Math.round(540 + 100 / state.unitY) });
});

test("fitting a portrait video overlay fills the portrait output canvas", async () => {
  const source = await readFile(new URL("../../src/lib/components/project/ProjectEditor.svelte", import.meta.url), "utf8");
  const ast = ts.createSourceFile("Editor.ts", source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1], ts.ScriptTarget.Latest, true);
  const fn = ast.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "filledElement");
  const state = { outputDimensions: { width: 1080, height: 1920 }, ELEMENT_SPACE: { width: 1920, height: 1080 }, ELEMENT_MIDDLE: { x: 960, y: 540 } };
  vm.runInContext(transpile(fn.getText(ast)), vm.createContext(state));
  const element = state.filledElement(9 / 16);
  const scale = 1080 / 1920;
  assert.equal(element.width * scale, 1080);
  assert.equal(element.width / (9 / 16) * scale, 1920);
  assert.equal(element.x * scale, 540);
  assert.equal(element.y * (1920 / 1080), 960);
});
