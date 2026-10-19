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

const text = (overrides = {}) => ({ ...api.newText([], api.TEXT_STYLES[0]), ...overrides });
const sticker = (overrides = {}) => ({ ...api.newSticker([], "/stickers/1f600.svg"), ...overrides });

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
  const tall = { ...api.newSticker([], "/a.png", 0.5, 400), x: 500, y: 500 };
  assert.equal(api.elementAt([tall], 500, 690, measure)?.id, tall.id);
  assert.equal(api.elementAt([tall], 650, 500, measure), null);
});

test("toLocal undoes the element's rotation", () => {
  const local = api.toLocal({ x: 100, y: 100, rotation: 90 }, 100, 150);
  close(local.x, 50);
  close(local.y, 0);
});

test("centers snap to the middle lines within the threshold", () => {
  assert.deepEqual(plain(api.snapToCenter(965, 300, 8)), { x: 960, y: 300, snapX: true, snapY: false });
  assert.deepEqual(plain(api.snapToCenter(100, 546, 8)), { x: 100, y: 540, snapX: false, snapY: true });
  assert.deepEqual(plain(api.snapToCenter(970, 530, 8)), { x: 970, y: 530, snapX: false, snapY: false });
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

test("new elements land in the middle, stepping past ones already there", () => {
  const first = api.newSticker([], "/stickers/1f600.svg");
  assert.deepEqual([first.id, first.x, first.y, first.size, first.aspect], [1, 960, 540, 280, 1]);
  const second = api.newText([first], api.TEXT_STYLES[1], "Kaiti SC");
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

test("backgrounds fit by covering or containing, centered", () => {
  assert.deepEqual(plain(api.fitRect(1080, 1920, "contain")), { x: 656.25, y: 0, width: 607.5, height: 1080 });
  const cover = api.fitRect(1080, 1920, "cover");
  assert.equal(cover.width, 1920);
  close(cover.height, (1920 * 1920) / 1080);
  close(cover.y, (1080 - cover.height) / 2);
  assert.deepEqual(plain(api.fitRect(1280, 720, "cover")), { x: 0, y: 0, width: 1920, height: 1080 });
});

test("reordering moves one element and clamps to the stack", () => {
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 0, 1)), ["b", "a", "c"]);
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 2, 5)), ["a", "b", "c"]);
  assert.deepEqual(plain(api.reorder(["a", "b", "c"], 1, -1)), ["b", "a", "c"]);
});

test("the homepage crop is the middle 4:3", () => {
  assert.deepEqual(plain(api.HOMEPAGE_CROP), { x: 240, y: 0, width: 1440, height: 1080 });
});
