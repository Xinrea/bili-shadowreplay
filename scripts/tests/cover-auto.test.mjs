import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function transpile(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}

async function loadModules() {
  const context = vm.createContext({});
  const elements = new vm.SourceTextModule(await transpile("../../src/lib/cover-elements.ts"), {
    context,
  });
  await elements.link(() => {
    throw new Error("Cover elements must have no runtime dependencies");
  });
  await elements.evaluate();
  const coverAuto = new vm.SourceTextModule(await transpile("../../src/lib/cover-auto.ts"), {
    context,
  });
  await coverAuto.link((specifier) => {
    if (specifier === "./cover-elements") return elements;
    throw new Error(`Unexpected import ${specifier}`);
  });
  await coverAuto.evaluate();
  return { coverAuto: coverAuto.namespace, elements: elements.namespace };
}

const { coverAuto, elements } = await loadModules();
// Every character is half the font size wide.
const measure = (text, font) => text.length * Number(/(\d+)px/.exec(font)[1]) * 0.5;

const WIDE = 1920;
const HOME_WIDTH = 1440;
const plain = (value) => JSON.parse(JSON.stringify(value));

const elementOf = (title, styleId, position) =>
  coverAuto.buildAutoCoverElements(title, styleId, position, measure)[0];

test("known style ids map onto their text styles", () => {
  assert.equal(coverAuto.coverStyleOf("tag-red").background, "#FF3B30");
  assert.equal(coverAuto.coverStyleOf("white").fill, "#FFFFFF");
});

test("unknown style ids fall back to the first preset", () => {
  const yellow = plain(elements.styleOf(elements.TEXT_STYLES[0]));
  assert.deepEqual(plain(coverAuto.coverStyleOf("made-up")), yellow);
  assert.deepEqual(plain(coverAuto.coverStyleOf("made-up")), plain(coverAuto.coverStyleOf("yellow")));
});

test("a short title keeps the largest font size and honors the position", () => {
  const element = elementOf("测试", "yellow", { x: 0.5, y: 0.78 });
  assert.equal(element.content, "测试");
  assert.equal(element.fontSize, coverAuto.AUTO_COVER_START_FONT_SIZE);
  assert.equal(element.x, WIDE / 2);
  assert.ok(Math.abs(element.y - 0.78 * 1080) < 1, `${element.y} at 78% height`);
});

test("a long title wraps to at most three lines and stays within the width cap", () => {
  const title = "这是一个特别特别长的标题用来测试自动换行与缩小字号的行为到底如何";
  for (const styleId of ["yellow", "tag-yellow", "white", "shadow"]) {
    const element = elementOf(title, styleId, { x: 0.5, y: 0.5 });
    const lines = element.content.split("\n");
    assert.ok(lines.length <= coverAuto.AUTO_COVER_MAX_LINES, `${styleId}: ${lines.length} lines`);
    const { width, height } = elements.layoutText(element, measure);
    assert.ok(width <= coverAuto.AUTO_COVER_MAX_WIDTH, `${styleId}: width ${width} over the cap`);
    assert.ok(height <= coverAuto.AUTO_COVER_MAX_HEIGHT, `${styleId}: height ${height} over the cap`);
    assert.ok(element.fontSize >= coverAuto.AUTO_COVER_MIN_FONT_SIZE);
  }
});

test("a blank title produces no elements", () => {
  const produced = coverAuto.buildAutoCoverElements("   ", "yellow", { x: 0.5, y: 0.5 }, measure);
  assert.equal(produced.length, 0);
});

test("an extreme position is clamped so the text stays fully visible", () => {
  const element = elementOf("标题", "yellow", { x: 0, y: 1 });
  const { width, height } = elements.layoutText(element, measure);
  const edge = (WIDE - HOME_WIDTH) / 2 + coverAuto.AUTO_COVER_MARGIN;
  assert.equal(element.x, edge + width / 2);
  assert.equal(element.y, 1080 - coverAuto.AUTO_COVER_MARGIN - height / 2);
});
