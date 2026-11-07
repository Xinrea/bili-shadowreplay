import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/MaterialPlaybackControls.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("controls.ts", script, ts.ScriptTarget.Latest, true);
const functions = parsed.statements.filter(ts.isFunctionDeclaration).map((node) => node.getText(parsed)).join("\n");
const effect = parsed.statements.find((node) =>
  ts.isExpressionStatement(node) && ts.isCallExpression(node.expression) && node.expression.expression.getText(parsed) === "$effect");
const transpile = (text) => ts.transpileModule(text, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText.replace(/^export /gm, "");
const code = transpile(functions);

function fixture(overrides = {}) {
  const video = {
    currentTime: 12, duration: 60, paused: true, ended: false,
    volume: 0.8, muted: false, playbackRate: 1, error: null,
    play: async () => { video.paused = false; },
    pause: () => { video.paused = true; },
  };
  const context = vm.createContext({
    video, duration: 60, currentTime: 12, canSeek: true,
    mediaError: "", actionError: "", fullscreen: false, fullscreenBusy: false,
    canFullscreen: true, TAURI_ENV: false, DOMException,
    disposed: false, ownsNativeFullscreen: false,
    ...overrides,
  });
  vm.runInContext(code, context);
  return context;
}

test("seeking clamps to real media duration and ignores invalid/unready requests", () => {
  const ctx = fixture();
  for (const [requested, expected] of [[-5, 0], [100, 60], [12.5, 12.5]]) {
    ctx.seek(requested);
    assert.equal(ctx.video.currentTime, expected);
    assert.equal(ctx.currentTime, expected);
  }
  ctx.seek(NaN);
  ctx.seek(Infinity);
  ctx.canSeek = false;
  ctx.seek(30);
  assert.equal(ctx.video.currentTime, 12.5);
});

test("mute restores a zero volume, preserves a nonzero volume, and slider unmutes", () => {
  const ctx = fixture();
  ctx.toggleMute();
  assert.equal(ctx.video.muted, true);
  assert.equal(ctx.video.volume, 0.8);
  ctx.toggleMute();
  assert.equal(ctx.video.muted, false);
  assert.equal(ctx.video.volume, 0.8);
  ctx.setVolume(0);
  ctx.toggleMute();
  assert.equal(ctx.video.volume, 0.5);
  ctx.toggleMute();
  ctx.setVolume(0.25);
  assert.equal(ctx.video.muted, false);
  assert.equal(ctx.video.volume, 0.25);
});

test("play/pause, replay and rejected play promises are handled", async () => {
  const ctx = fixture();
  await ctx.togglePlayback();
  assert.equal(ctx.video.paused, false);
  await ctx.togglePlayback();
  assert.equal(ctx.video.paused, true);
  ctx.video.ended = true;
  await ctx.togglePlayback();
  assert.equal(ctx.video.currentTime, 0);
  ctx.video.paused = true;
  ctx.video.play = async () => { throw new Error("blocked"); };
  await ctx.togglePlayback();
  assert.match(ctx.actionError, /播放未能开始/);
  ctx.video.play = async () => { throw new DOMException("interrupted", "AbortError"); };
  await ctx.togglePlayback();
  assert.equal(ctx.actionError, "");
});

test("media events synchronize external seeks, rates, end, errors and remove listeners", () => {
  const ctx = fixture();
  const listeners = new Map();
  ctx.video.addEventListener = (event, callback) => listeners.set(event, callback);
  ctx.video.removeEventListener = (event, callback) => {
    assert.equal(listeners.get(event), callback);
    listeners.delete(event);
  };
  const cleanup = vm.runInContext(transpile(`(${effect.expression.arguments[0].getText(parsed)})()`), ctx);
  assert.equal(ctx.currentTime, 12);
  ctx.video.currentTime = 42;
  listeners.get("timeupdate")();
  assert.equal(ctx.currentTime, 42);
  ctx.video.playbackRate = 2;
  listeners.get("ratechange")();
  assert.equal(ctx.playbackRate, 2);
  ctx.video.ended = true;
  listeners.get("ended")();
  assert.equal(ctx.ended, true);
  listeners.get("waiting")();
  assert.equal(ctx.waiting, true);
  listeners.get("seeked")();
  assert.equal(ctx.waiting, false);
  for (const value of [NaN, Infinity, 0]) {
    ctx.video.duration = value;
    listeners.get("durationchange")();
    assert.equal(ctx.duration, 0);
  }
  ctx.video.error = { code: 4 };
  listeners.get("error")();
  assert.match(ctx.mediaError, /无法播放/);
  cleanup();
  assert.equal(listeners.size, 0);
});

test("shortcuts stay within the player and preserve native input and button activation", () => {
  class Element {
    constructor(inside, input = false) { this.inside = inside; this.input = input; }
    closest() { return this.input; }
  }
  const ctx = fixture({ Element, container: { contains: (target) => target.inside } });
  const invokeKey = (key, target, extra = {}) => {
    let prevented = false;
    ctx.onKeydown({ key, target, preventDefault: () => { prevented = true; }, ...extra });
    return prevented;
  };
  assert.equal(invokeKey("ArrowRight", new Element(false)), false);
  assert.equal(invokeKey("ArrowRight", new Element(true, true)), false);
  assert.equal(invokeKey(" ", new Element(true)), false);
  assert.equal(invokeKey("Enter", new Element(true)), false);
  assert.equal(invokeKey("m", new Element(true), { ctrlKey: true }), false);
  assert.equal(invokeKey("ArrowRight", new Element(true)), true);
  assert.equal(ctx.video.currentTime, 17);
  assert.equal(invokeKey("m", new Element(true)), true);
  assert.equal(ctx.video.muted, true);
});

test("web fullscreen targets the whole player and reports failures without sticking busy", async () => {
  let entered = false;
  let exited = false;
  const container = { requestFullscreen: async () => { entered = true; } };
  const document = { fullscreenElement: null, exitFullscreen: async () => { exited = true; } };
  const ctx = fixture({ container, document });
  await ctx.toggleFullscreen();
  assert.equal(entered, true);
  document.fullscreenElement = container;
  ctx.syncFullscreen();
  assert.equal(ctx.fullscreen, true);
  await ctx.toggleFullscreen();
  assert.equal(exited, true);
  document.fullscreenElement = null;
  ctx.syncFullscreen();
  assert.equal(ctx.fullscreen, false);
  container.requestFullscreen = async () => { throw new Error("denied"); };
  await ctx.toggleFullscreen();
  assert.match(ctx.actionError, /无法切换全屏/);
  assert.equal(ctx.fullscreenBusy, false);
});

test("desktop fullscreen uses the native window and Escape exits even outside the player", async () => {
  let nativeFullscreen = false;
  const window = {
    isFullscreen: async () => nativeFullscreen,
    setFullscreen: async (value) => { nativeFullscreen = value; },
  };
  const ctx = fixture({ TAURI_ENV: true, container: {}, getCurrentWindow: () => window });
  await ctx.toggleFullscreen();
  assert.equal(nativeFullscreen, true);
  assert.equal(ctx.fullscreen, true);
  let prevented = false;
  ctx.onKeydown({ key: "Escape", preventDefault: () => { prevented = true; } });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(prevented, true);
  assert.equal(nativeFullscreen, false);
  assert.equal(ctx.fullscreen, false);
});

test("native cleanup exits only fullscreen entered by these controls, including pending entry", async () => {
  let nativeFullscreen = true;
  const window = {
    isFullscreen: async () => nativeFullscreen,
    setFullscreen: async (value) => { nativeFullscreen = value; },
  };
  const ctx = fixture({ TAURI_ENV: true, container: {}, getCurrentWindow: () => window });
  await ctx.restoreNativeFullscreen();
  assert.equal(nativeFullscreen, true, "pre-existing fullscreen is preserved");
  nativeFullscreen = false;
  await ctx.toggleFullscreen();
  await ctx.restoreNativeFullscreen();
  assert.equal(nativeFullscreen, false);
  window.setFullscreen = async (value) => {
    nativeFullscreen = value;
    if (value) ctx.disposed = true;
  };
  await ctx.toggleFullscreen();
  assert.equal(nativeFullscreen, false, "unmount during an in-flight entry is restored");
});
