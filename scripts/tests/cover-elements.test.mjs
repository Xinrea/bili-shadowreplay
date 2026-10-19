import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/cover-elements.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("Cover elements must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

const api = await loadModule();
const plain = (value) => JSON.parse(JSON.stringify(value));
// Every character is half the font size wide.
const measure = (text, font) => text.length * Number(/(\d+)px/.exec(font)[1]) * 0.5;
const close = (actual, expected) => assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} ≈ ${expected}`);

const middle = { x: 960, y: 540 };
const text = (overrides = {}) => ({ ...api.newText([], middle, api.TEXT_STYLES[0]), ...overrides });
const sticker = (overrides = {}) => ({ ...api.newSticker([], middle, "/stickers/1f600.svg"), ...overrides });

test("text is as wide as its widest line plus the outline", () => {
  const t = text({ content: "ab\nabcd", fontSize: 100, stroke: "#000", strokeWidth: 0.1, background: null });
  const layout = api.layoutText(t, measure);
  assert.deepEqual(plain(layout.lines), ["ab", "abcd"]);
  assert.equal(layout.textWidth, 200);
  assert.equal(layout.textHeight, 2 * 100 * api.LINE_HEIGHT);
  close(layout.width, 200 + 2 * (10 + 5));
  close(layout.height, 250 + 2 * 10);
  assert.match(layout.font, /^bold 100px "PingFang SC"/);
});

test("a background box pads the text instead of the outline", () => {
  const t = text({ content: "abcd", fontSize: 100, stroke: null, background: "#FFE14D" });
  const layout = api.layoutText(t, measure);
  close(layout.width, 200 + 70);
  close(layout.height, 125 + 24);
});

test("empty text keeps a width so it can still be grabbed", () => {
  const layout = api.layoutText(text({ content: "", fontSize: 80, stroke: null }), measure);
  assert.equal(layout.textWidth, 40);
});

test("a chosen font comes first with the default stack behind it", () => {
  assert.equal(api.fontFamily(""), api.DEFAULT_FONT_STACK);
  assert.equal(api.fontFamily('Odd "Name"'), `"Odd \\"Name\\"", ${api.DEFAULT_FONT_STACK}`);
});

test("hit testing follows rotation and picks the topmost element", () => {
  const below = sticker({ id: 1, x: 500, y: 500, size: 200 });
  const above = sticker({ id: 2, x: 560, y: 500, size: 200 });
  assert.equal(api.elementAt([below, above], 590, 500, measure).id, 2);
  assert.equal(api.elementAt([below, above], 420, 500, measure).id, 1);
  assert.equal(api.elementAt([below, above], 700, 700, measure), null);

  // A 400×100 bar turned upright covers points above its center, not beside it.
  const bar = text({ id: 3, x: 1000, y: 500, content: "abcdefgh", fontSize: 100, stroke: null, rotation: 90 });
  const { width } = api.elementSize(bar, measure);
  assert.equal(api.elementAt([bar], 1000, 500 - width / 2 + 5, measure)?.id, 3);
  assert.equal(api.elementAt([bar], 1000 + width / 2 - 5, 500, measure), null);
  assert.equal(api.elementAt([bar], 1060, 500, measure, 20)?.id, 3);
});

test("a sticker's size is its longer side", () => {
  assert.deepEqual(plain(api.stickerSize({ size: 400, aspect: 2 })), { width: 400, height: 200 });
  assert.deepEqual(plain(api.stickerSize({ size: 400, aspect: 0.5 })), { width: 200, height: 400 });
  // A tall picture is hit only within its own width.
  const tall = { ...api.newSticker([], middle, "/a.png", 0.5, 400), x: 500, y: 500 };
  assert.equal(api.elementAt([tall], 500, 690, measure)?.id, tall.id);
  assert.equal(api.elementAt([tall], 650, 500, measure), null);
});

test("toLocal undoes the element's rotation", () => {
  const local = api.toLocal({ x: 100, y: 100, rotation: 90 }, 100, 150);
  close(local.x, 50);
  close(local.y, 0);
});

test("centers snap to the middle lines within the threshold", () => {
  assert.deepEqual(plain(api.snapToCenter(965, 300, middle, 8)), { x: 960, y: 300, snapX: true, snapY: false });
  assert.deepEqual(plain(api.snapToCenter(100, 546, middle, 8)), { x: 100, y: 540, snapX: false, snapY: true });
  assert.deepEqual(plain(api.snapToCenter(970, 530, middle, 8)), { x: 970, y: 530, snapX: false, snapY: false });
  // The middle of a cover that is zoomed and moved.
  assert.deepEqual(plain(api.snapToCenter(1203, 402, { x: 1200, y: 400 }, 8)), { x: 1200, y: 400, snapX: true, snapY: true });
});

test("rotation wraps, sticks to right angles and follows a step", () => {
  assert.equal(api.snapRotation(3), 0);
  assert.equal(api.snapRotation(-2), 0);
  assert.equal(api.snapRotation(10), 10);
  assert.equal(api.snapRotation(268), -90);
  assert.equal(api.snapRotation(370), 10);
  assert.equal(api.snapRotation(52, 15), 45);
  assert.equal(api.snapRotation(-179), -180);
});

test("scaling keeps sizes within limits", () => {
  assert.deepEqual(plain(api.scaledSize(text({ fontSize: 100 }), 1.5)), { fontSize: 150 });
  assert.deepEqual(plain(api.scaledSize(text({ fontSize: 100 }), 0.01)), { fontSize: 12 });
  assert.deepEqual(plain(api.scaledSize(sticker({ size: 280 }), 0.5)), { size: 140 });
  assert.deepEqual(plain(api.scaledSize(sticker({ size: 280 }), 100)), { size: 1600 });
});

test("new elements land where asked, stepping past ones already there", () => {
  const first = api.newSticker([], middle, "/stickers/1f600.svg");
  assert.deepEqual([first.id, first.x, first.y, first.size, first.aspect], [1, 960, 540, 280, 1]);
  const second = api.newText([first], middle, api.TEXT_STYLES[1], "Kaiti SC");
  assert.deepEqual([second.id, second.x, second.y, second.font], [2, 1020, 600, "Kaiti SC"]);
  assert.equal(second.fill, api.TEXT_STYLES[1].fill);
  assert.equal("label" in second, false);
});

test("styles compare by look, ignoring the width of a missing outline", () => {
  const preset = api.TEXT_STYLES.find((s) => s.id === "tag-red");
  assert.ok(api.sameStyle({ ...preset, fill: "#ffffff", strokeWidth: 0.2 }, preset));
  assert.ok(!api.sameStyle({ ...preset, shadow: true }, preset));
  const outlined = api.TEXT_STYLES[0];
  assert.ok(!api.sameStyle({ ...outlined, strokeWidth: 0.2 }, outlined));
});

test("pictures fit by covering or containing a target, centered", () => {
  const target = { width: 1920, height: 1080 };
  assert.deepEqual(plain(api.fitRect(1080, 1920, target, "contain")), { x: 656.25, y: 0, width: 607.5, height: 1080 });
  const cover = api.fitRect(1080, 1920, target, "cover");
  assert.equal(cover.width, 1920);
  close(cover.height, (1920 * 1920) / 1080);
  close(cover.y, (1080 - cover.height) / 2);
  assert.deepEqual(plain(api.fitRect(1280, 720, target, "cover")), { x: 0, y: 0, width: 1920, height: 1080 });
});

test("reordering moves one element and clamps to the stack", () => {
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 0, 1)), ["b", "a", "c"]);
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 2, 5)), ["a", "b", "c"]);
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 1, -1)), ["b", "a", "c"]);
});

const home = api.aspectOf("home");
const wide = api.aspectOf("wide");

test("covers are saved at Bilibili's 4:3 and 16:9 sizes", () => {
  assert.deepEqual(plain(api.COVER_SIZES), { home: { width: 1440, height: 1080 }, wide: { width: 1920, height: 1080 } });
  assert.deepEqual(plain(api.sceneOf()), { width: 1920, height: 1080 });
  assert.deepEqual(plain(api.sceneOf(640, 480)), { width: 1440, height: 1080 });
});

test("by default each cover covers the picture, centered", () => {
  const scene = api.sceneOf(1920, 1080);
  assert.deepEqual(plain(api.defaultFrame(scene, wide)), { x: 960, y: 540, zoom: 1 });
  assert.deepEqual(plain(api.frameRect(scene, wide, api.defaultFrame(scene, wide))), { x: 0, y: 0, width: 1920, height: 1080 });
  // The homepage shows the middle 4:3, as Bilibili crops it.
  assert.deepEqual(plain(api.frameRect(scene, home, api.defaultFrame(scene, home))), { x: 240, y: 0, width: 1440, height: 1080 });
});

test("a portrait picture is shown whole on a fill rather than cropped to a strip", () => {
  const scene = api.sceneOf(1080, 1920);
  const frame = api.defaultFrame(scene, wide);
  assert.equal(frame.zoom, api.minZoom(scene, wide));
  const rect = api.frameRect(scene, wide, frame);
  close(rect.height, 1080);
  close(rect.x + rect.width / 2, scene.width / 2);
});

test("frames stay over the picture and within the zoom range", () => {
  const scene = api.sceneOf(1920, 1080);
  // Zoomed in, the frame cannot leave the picture.
  const moved = api.clampFrame(scene, wide, { x: 5000, y: -100, zoom: 2 });
  assert.deepEqual(plain(api.frameRect(scene, wide, moved)), { x: 960, y: 0, width: 960, height: 540 });
  // Zoomed out past covering, it stays centered on the picture.
  const out = api.clampFrame(scene, home, { x: 0, y: 0, zoom: 0.1 });
  assert.equal(out.zoom, api.minZoom(scene, home));
  assert.deepEqual([out.x, out.y], [960, 540]);
  assert.equal(api.clampFrame(scene, wide, { x: 960, y: 540, zoom: 99 }).zoom, api.MAX_ZOOM);
});

test("zooming keeps the point under the pointer in place", () => {
  const scene = api.sceneOf(1920, 1080);
  const start = api.defaultFrame(scene, wide);
  const anchor = { x: 480, y: 270 };
  const before = api.frameRect(scene, wide, start);
  const zoomed = api.zoomFrame(scene, wide, start, 2, anchor);
  const after = api.frameRect(scene, wide, zoomed);
  close((anchor.x - before.x) / before.width, (anchor.x - after.x) / after.width);
  close((anchor.y - before.y) / before.height, (anchor.y - after.y) / after.height);
  assert.equal(zoomed.zoom, 2);
});

test("elements sit on the covers: the 4:3 cover shows the middle of the 16:9 one", () => {
  assert.deepEqual(plain(api.elementArea("wide")), { x: 0, y: 0, width: 1920, height: 1080 });
  assert.deepEqual(plain(api.elementArea("home")), { x: 240, y: 0, width: 1440, height: 1080 });
});
