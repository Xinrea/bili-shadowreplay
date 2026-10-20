import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadInvoker({ tauri, endpoint = "" }) {
  const tauriCalls = [];
  const fetches = [];
  const context = vm.createContext({
    window: tauri ? { __TAURI_INTERNALS__: {} } : {},
    localStorage: { getItem: (key) => (key === "endpoint" ? endpoint : null) },
    fetch: async (url) => {
      fetches.push(url);
      throw new Error("static URLs must not need an HTTP request");
    },
  });
  const dependencies = {
    "@tauri-apps/api/core": {
      invoke: async (command) => {
        tauriCalls.push(command);
        if (command === "get_static_port") return 41234;
        throw new Error(`unexpected command ${command}`);
      },
    },
    "@tauri-apps/plugin-http": { fetch: async () => {} },
    "@tauri-apps/plugin-shell": { open: async () => {} },
    "@tauri-apps/plugin-deep-link": { onOpenUrl: async () => {} },
    "./events": { createEventClient: () => ({ listen: async () => () => {} }) },
  };
  const source = await readFile(new URL("../../src/lib/invoker.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context });
  // Keep each synthetic module referenced until evaluation finishes; see
  // updater.test.mjs.
  const linked = new Map();
  await module.link((name) => {
    const exports = dependencies[name];
    if (!exports) throw new Error(`unexpected import ${name}`);
    if (!linked.has(name)) {
      linked.set(name, new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
      }, { context }));
    }
    return linked.get(name);
  });
  await module.evaluate();
  return { invoker: module.namespace, tauriCalls, fetches };
}

test("desktop static URLs ask only for the static port, once", async () => {
  const { invoker, tauriCalls } = await loadInvoker({ tauri: true });
  assert.equal(
    await invoker.get_static_url("cache", "bilibili/1/L1/cover.jpg"),
    "http://localhost:41234/cache/bilibili/1/L1/cover.jpg",
  );
  assert.equal(
    await invoker.get_static_url("output", "clip.mp4"),
    "http://localhost:41234/output/clip.mp4",
  );
  assert.deepEqual(tauriCalls, ["get_static_port"]);
});

test("web static URLs follow the configured endpoint without a request", async () => {
  const { invoker, fetches } = await loadInvoker({ tauri: false, endpoint: "https://bsr.example/base/" });
  assert.equal(
    await invoker.get_static_url("cache", "youtube/UCx/L1/cover.jpg"),
    "https://bsr.example/base/cache/youtube/UCx/L1/cover.jpg",
  );
  assert.deepEqual(fetches, []);
});

test("each path segment is URL-encoded but the separators are kept", async () => {
  const { invoker } = await loadInvoker({ tauri: false, endpoint: "http://host:3000" });
  assert.equal(
    await invoker.get_static_url("output", "直播 #1/a?b.mp4"),
    "http://host:3000/output/%E7%9B%B4%E6%92%AD%20%231/a%3Fb.mp4",
  );
});
