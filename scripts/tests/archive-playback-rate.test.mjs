import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const presets = await readFile(new URL("../../src/lib/playback-rates.ts", import.meta.url), "utf8");
const player = await readFile(new URL("../../src/lib/components/Player.svelte", import.meta.url), "utf8");
const script = player.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("Player.ts", script, ts.ScriptTarget.Latest, true);
const setter = parsed.statements.find((node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === "setPlaybackRate");
// Exercise the actual exported player method with a media-element stand-in.
const code = ts.transpileModule(`${presets}\n${setter.getText(parsed)}`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText.replace(/^export /gm, "");

function fixture(video = { playbackRate: 1 }) {
  const context = vm.createContext({ video, playback_rate: 1 });
  vm.runInContext(code, context);
  return {
    context,
    rates: vm.runInContext("Array.from(PLAYBACK_RATES)", context),
    set: (rate) => {
      context.rate = rate;
      vm.runInContext("setPlaybackRate(rate)", context);
    },
  };
}

test("archive playback keeps the historical speed presets", () => {
  assert.deepEqual(Array.from(fixture().rates), [0.5, 1, 1.5, 2, 5]);
});

test("every speed preset updates the media element and the exposed state", () => {
  const f = fixture();
  for (const rate of [...f.rates, 1]) {
    f.set(rate);
    assert.equal(f.context.video.playbackRate, rate);
    assert.equal(f.context.playback_rate, rate);
  }
});

test("unsupported rates leave playback unchanged", () => {
  const f = fixture();
  f.set(2);
  for (const rate of [0, -1, NaN, Infinity, 1.25, "2"]) {
    f.set(rate);
    assert.equal(f.context.video.playbackRate, 2);
    assert.equal(f.context.playback_rate, 2);
  }
});

test("speed changes before the video is ready are harmless", () => {
  const f = fixture(null);
  f.set(2);
  assert.equal(f.context.playback_rate, 1);
});

test("player shortcuts do not steal keyboard events from settings buttons", async () => {
  let handler;
  function visit(node) {
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(parsed) === "document.addEventListener" &&
      node.arguments[0]?.text === "keydown"
    ) {
      handler = node.arguments[1];
    }
    ts.forEachChild(node, visit);
  }
  visit(parsed);
  assert.ok(handler);
  const handlerCode = ts.transpileModule(`const keydown = ${handler.getText(parsed)}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const context = vm.createContext({
    target: {
      tagName: "BUTTON",
      closest(selector) { return selector === "button" ? this : null; },
    },
    preventDefault() { assert.fail("Native button keys must not be prevented"); },
  });
  vm.runInContext(handlerCode, context);
  for (const key of ["Tab", "Enter", " ", "[", "]", "Backspace"]) {
    context.key = key;
    await vm.runInContext("keydown({ key, target, preventDefault })", context);
  }
});
