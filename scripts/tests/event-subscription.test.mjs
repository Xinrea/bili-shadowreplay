import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const settle = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

async function fixture() {
  const timers = new Map();
  let id = 0;
  const context = vm.createContext({
    AbortController,
    setTimeout: (callback, ms) => { assert.equal(ms, 3_000); timers.set(++id, callback); return id; },
    clearTimeout: (id) => timers.delete(id),
  });
  const source = await readFile(new URL("../../src/lib/event-subscription.ts", import.meta.url), "utf8");
  const output = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } }).outputText;
  const module = new vm.SourceTextModule(output, { context });
  await module.link(() => { throw new Error("Unexpected dependency"); });
  await module.evaluate();
  return { keep: module.namespace.keepEventSubscription, timers, retry: async () => {
    const callbacks = [...timers.values()]; timers.clear();
    callbacks.forEach((callback) => callback()); await settle();
  } };
}

test("setup is nonblocking and retries failure without reinitializing the owner", async () => {
  const f = await fixture();
  const controller = new AbortController();
  let attempts = 0, cleanups = 0;
  const errors = [];
  const stop = f.keep(async () => {
    if (++attempts === 1) throw new Error("ACK timeout");
    return () => { cleanups++; };
  }, controller.signal, (error) => errors.push(error));
  assert.equal(typeof stop, "function");
  await settle();
  assert.equal(errors.length, 1);
  assert.equal(f.timers.size, 1);
  await f.retry();
  assert.equal(attempts, 2);
  assert.equal(f.timers.size, 0);
  stop(); stop();
  assert.equal(cleanups, 1);
});

test("destroying the owner cancels retry timers", async () => {
  const f = await fixture();
  const controller = new AbortController();
  let attempts = 0;
  f.keep(async () => { attempts++; throw new Error("offline"); }, controller.signal, () => {});
  await settle();
  controller.abort();
  assert.equal(f.timers.size, 0);
  await f.retry();
  assert.equal(attempts, 1);
});

test("stopping pending setup aborts it and cleans up a late successful setup", async () => {
  const f = await fixture();
  const controller = new AbortController();
  let resolve, subscriptionSignal, cleanups = 0;
  const pending = new Promise((done) => { resolve = done; });
  const stop = f.keep((signal) => { subscriptionSignal = signal; return pending; }, controller.signal, () => {});
  stop();
  assert.equal(subscriptionSignal.aborted, true);
  resolve(() => { cleanups++; });
  await settle();
  assert.equal(cleanups, 1);
  assert.equal(f.timers.size, 0);
});

test("aborted owners never start subscription setup", async () => {
  const f = await fixture();
  const controller = new AbortController();
  controller.abort();
  f.keep(() => { throw new Error("Must not start"); }, controller.signal, () => {});
  assert.equal(f.timers.size, 0);
});
