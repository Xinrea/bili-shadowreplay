import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";

test("OBS URLs use the backend host for wildcard listeners and bracket IPv6", async () => {
  const source = await readFile(new URL("../../src/lib/rtmp.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("Unexpected runtime dependency"); });
  await module.evaluate();
  const url = (listen_addr, host = "192.168.1.2") => module.namespace.rtmpServerUrl({ listen_addr, port: 1935 }, host);
  assert.equal(url("0.0.0.0"), "rtmp://192.168.1.2:1935/live");
  assert.equal(url("0.0.0.0", ""), "rtmp://127.0.0.1:1935/live");
  assert.equal(url("127.0.0.1"), "rtmp://127.0.0.1:1935/live");
  assert.equal(url("::", "2001:db8::1"), "rtmp://[2001:db8::1]:1935/live");
  assert.equal(url("::", "[::1]"), "rtmp://[::1]:1935/live");
  assert.equal(url("::1"), "rtmp://[::1]:1935/live");
});

test("embedded RTMP settings expose the allocated endpoint and copyable credentials", async () => {
  const context = vm.createContext({
    localStorage: { getItem: () => null },
    window: { __TAURI_INTERNALS__: {} },
  });
  const source = await readFile(new URL("../../src/lib/rtmp.ts", import.meta.url), "utf8");
  const rtmp = new vm.SourceTextModule(ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText, { context });
  await rtmp.link(() => { throw new Error("Unexpected RTMP dependency"); });
  await rtmp.evaluate();
  const componentSource = await readFile(new URL("../../src/lib/components/RtmpIngestSettings.svelte", import.meta.url), "utf8");
  const component = new vm.SourceTextModule(compile(componentSource, { generate: "server" }).js.code, { context });
  await component.link(async (name) => {
    if (name === "../rtmp") return rtmp;
    if (name === "lucide-svelte") {
      // Icons do not affect room state; the package requires Vite resolution.
      return new vm.SyntheticModule(["Copy", "X"], function () {
        this.setExport("Copy", () => {});
        this.setExport("X", () => {});
      }, { context });
    }
    const exports = await import(name);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await component.evaluate();
  const info = {
    room_id: "rtmp-test", name: "我的 OBS", listen_addr: "0.0.0.0", port: 52137,
    stream_key: "generated-test-key", listening: true, live: false, recording: false,
  };
  const show = () => render(component.namespace.default, {
    props: { info },
  }).body;
  const waiting = show();
  assert.match(waiting, /rtmp:\/\/127\.0\.0\.1:52137\/live/);
  assert.match(waiting, /generated-test-key/);
  assert.equal((waiting.match(/readonly/g) ?? []).length, 2);
  assert.match(waiting, /复制推流地址/);
  assert.match(waiting, /复制推流密钥/);
  info.listening = false;
  assert.match(show(), /推流服务未启动/);
});
