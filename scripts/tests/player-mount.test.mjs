import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/Player.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("Player.ts", script, ts.ScriptTarget.Latest, true);
const mount = parsed.statements.find(node =>
  ts.isFunctionDeclaration(node) && node.name?.text === "mountPlayer");
const hook = name => parsed.statements.find(node =>
  ts.isExpressionStatement(node) && ts.isCallExpression(node.expression) &&
  node.expression.expression.getText(parsed) === name).expression.arguments[0].getText(parsed);
const code = ts.transpileModule(`
  ${mount.getText(parsed)}
  globalThis.mount = ${hook("onMount")};
  globalThis.dispose = ${hook("onDestroy")};
`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const settle = () => new Promise(resolve => setImmediate(resolve));

function fixture(attach = async () => {}) {
  const calls = [];
  const video = { parentElement: {} };
  class Player {
    async attach(element) {
      assert.equal(element, video);
      calls.push("attach");
      await attach();
      calls.push("attached");
    }
    async destroy() { calls.push("destroy-player"); }
  }
  class Overlay {
    constructor(player, container, element) {
      assert.equal(container, video.parentElement);
      assert.equal(element, video);
      this.player = player;
      calls.push("overlay");
    }
    async destroy() {
      calls.push("destroy-ui");
      await this.player.destroy();
    }
  }
  const state = {
    destroyed: false, shaka_player: undefined, shaka_ui: undefined, video,
    intervals: [1, 2],
    shaka: { Player, ui: { Overlay }, polyfill: { installAll: () => calls.push("polyfill") } },
    init: async () => calls.push("init"),
    log: { error: (...args) => calls.push(["error", ...args]) },
    alert: message => calls.push(["alert", message]),
    danmuStyle: { stop() {} }, danmuSubscription: { abort() {} },
    stopDanmu: undefined, cancelDanmuBatch: undefined, floatingOverlay: undefined,
    clearInterval: id => calls.push(["clear-interval", id]),
    // The global page-load event already fired; there is intentionally no event
    // source here. A late mount must initialize entirely from the component hook.
  };
  vm.runInContext(code, vm.createContext(state));
  return { state, calls };
}

test("a component mounted after the page-load event creates, attaches and initializes its own player", async () => {
  const { state, calls } = fixture();
  state.mount();
  await settle();
  assert.deepEqual(calls, ["polyfill", "overlay", "attach", "attached", "init"]);
  // Opt out of Shaka's one-shot auto scan to avoid an early-mount double player.
  assert.doesNotMatch(source, /data-shaka-player|addEventListener\("shaka-ui-loaded"/);
  assert.match(source, /bind:this=\{video\}/);
});

test("unmount during attach destroys the owned player and never initializes detached controls", async () => {
  let finish;
  const pending = new Promise(resolve => { finish = resolve; });
  const { state, calls } = fixture(() => pending);
  state.mount();
  state.dispose();
  finish();
  await settle();
  assert.equal(state.destroyed, true);
  assert.equal(calls.includes("init"), false);
  assert.equal(calls.filter(call => call === "destroy-ui").length, 1);
  assert.equal(calls.filter(call => call === "destroy-player").length, 1);
  assert.ok(calls.some(call => Array.isArray(call) && call[0] === "clear-interval"));
  assert.equal(calls.some(call => Array.isArray(call) && call[0] === "alert"), false);
});

test("availability remount creates a fresh player without another global Shaka event", async () => {
  const first = fixture();
  first.state.mount();
  await settle();
  first.state.dispose();
  const second = fixture();
  second.state.mount();
  await settle();
  assert.notEqual(first.state.shaka_player, second.state.shaka_player);
  assert.ok(second.calls.includes("init"));
});

test("attach errors are reported but expected cancellation during unmount is silent", async () => {
  const failed = fixture(async () => { throw new Error("attach failed"); });
  failed.state.mount();
  await settle();
  assert.equal(failed.calls.includes("init"), false);
  assert.ok(failed.calls.some(call => Array.isArray(call) && call[0] === "alert"));

  let reject;
  const pending = new Promise((_, fail) => { reject = fail; });
  const cancelled = fixture(() => pending);
  cancelled.state.mount();
  cancelled.state.dispose();
  reject(new Error("player destroyed"));
  await settle();
  assert.equal(cancelled.calls.some(call => Array.isArray(call) && call[0] === "alert"), false);
});
