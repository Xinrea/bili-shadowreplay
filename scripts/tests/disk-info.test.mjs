import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";

const source = await readFile(new URL("../../src/lib/components/DiskUsagePanel.svelte", import.meta.url), "utf8");
const transpile = code => ts.transpileModule(code, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const sizes = new vm.SourceTextModule(transpile(await readFile(new URL("../../src/lib/size-format.ts", import.meta.url), "utf8")));
await sizes.link(() => { throw new Error("Unexpected formatter dependency"); });
await sizes.evaluate();

const pool = (mount, total, free) => ({
  id: `mount:${mount}`, name: mount, mount_points: [mount], total, free,
});

async function renderPanel({ disks = [], loading = false, error = "" } = {}) {
  const seeded = source
    .replace("let disks: DiskInfo[] = $state([]);", `let disks: DiskInfo[] = $state(${JSON.stringify(disks)});`)
    .replace("let loading = $state(true);", `let loading = $state(${loading});`)
    .replace('let error = $state("");', `let error = $state(${JSON.stringify(error)});`);
  const component = new vm.SourceTextModule(compile(seeded, { generate: "server" }).js.code);
  await component.link(async name => {
    const exports = name === "../size-format" ? sizes.namespace
      : name === "../invoker" ? { invoke() { throw new Error("SSR must not query the host"); } }
      : name === "../state-refresh" ? { createStateRefresh: () => ({ run() {} }), watchStateChanges() {} }
      : await import(name);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    });
  });
  await component.evaluate();
  return render(component.namespace.default).body;
}

function loader(invoke) {
  const state = { disks: [], loading: false, error: "", disposed: false, invoke };
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("panel.ts", script, ts.ScriptTarget.Latest, true);
  const functions = parsed.statements.filter(ts.isFunctionDeclaration).map(node => node.getText(parsed)).join("\n");
  vm.runInContext(transpile(functions), vm.createContext(state));
  return state;
}

test("local disk panel renders separate mount rows without adding their capacities", async () => {
  const body = await renderPanel({ disks: [
    pool("/", 1024, 256),
    pool("/Volumes/Recordings & Backup", 1024, 0),
  ] });
  assert.match(body, /本机磁盘/);
  assert.match(body, /256 B<\/span> 可用/);
  assert.match(body, /共 1.00 KiB/);
  assert.match(body, /75%/);
  assert.match(body, /100%/);
  assert.match(body, /aria-valuetext="已用 768 B，总量 1.00 KiB，可用 256 B"/);
  assert.equal((body.match(/<progress /g) || []).length, 2);
  assert.match(body, /\/Volumes\/Recordings &amp; Backup/);
  assert.doesNotMatch(body, /<button|刷新|重试|S3|按挂载点|存储池|容量不应相加|2.00 KiB|NaN|Infinity/);
});

test("shared APFS capacity renders once with a name and mount details rather than duplicate rows", async () => {
  const body = await renderPanel({ disks: [{
    id: "apfs:disk3", name: "Macintosh HD",
    mount_points: ["/", "/System/Volumes/Data"], total: 1024, free: 256,
  }] });
  assert.equal((body.match(/<progress /g) || []).length, 1);
  assert.match(body, />Macintosh HD<\/p>/);
  assert.match(body, /title="\/\n\/System\/Volumes\/Data"/);
  assert.doesNotMatch(body, />\/System\/Volumes\/Data<\/p>/);
  assert.match(source, /\{#each disks as disk \(disk.id\)\}/);
  const sameNames = await renderPanel({ disks: [
    { ...pool("/a", 1024, 256), name: "Archive" },
    { ...pool("/b", 1024, 256), name: "Archive" },
  ] });
  assert.equal((sameNames.match(/<progress /g) || []).length, 2);
});

test("empty, loading and failed disk snapshots stay concise without manual refresh controls", async () => {
  assert.match(await renderPanel(), /暂无磁盘信息/);
  const loading = await renderPanel({ loading: true });
  assert.match(loading, /读取中…/);
  assert.match(loading, /aria-busy="true"/);
  const failed = await renderPanel({ error: "磁盘信息暂不可用" });
  assert.match(failed, /role="status"/);
  assert.match(failed, /磁盘信息暂不可用/);
  assert.doesNotMatch(failed + loading, /<button|<progress|刷新|重试/);
  assert.match(source, /watchStateChanges\(refresh, \["records-changed"\]\)/, "automatic refresh remains enabled");
});

test("zero or over-reported free space cannot render invalid progress", async () => {
  for (const disk of [
    pool("/", 0, 0),
    pool("/", 10, 20),
  ]) {
    const body = await renderPanel({ disks: [disk] });
    assert.match(body, /value="0"/);
    assert.doesNotMatch(body, /NaN|Infinity|已用 -/);
  }
});

test("disk loading errors clear stale data and retries only query the host endpoint", async () => {
  const state = loader(async command => {
    assert.equal(command, "get_disk_info");
    throw new Error("host probe failed");
  });
  state.disks = [pool("/stale", 100, 50)];
  await state.loadDisks();
  assert.equal(state.disks.length, 0);
  assert.equal(state.loading, false);
  assert.match(state.error, /暂不可用/);
  const snapshot = [pool("/", 100, 40), pool("/data", 200, 90)];
  state.invoke = async command => {
    assert.equal(command, "get_disk_info");
    return snapshot;
  };
  await state.loadDisks();
  assert.equal(state.disks, snapshot);
  assert.equal(state.error, "");
  assert.equal(state.loading, false);
});

test("unmounted disk panels ignore in-flight results and clean up the refresh watcher", async () => {
  let resolve;
  const state = loader(() => new Promise(done => { resolve = done; }));
  const pending = state.loadDisks();
  state.disposed = true;
  resolve([pool("/", 100, 40)]);
  await pending;
  assert.equal(state.disks.length, 0);
  assert.match(source, /disposed = true; stop\(\)/);
});

test("summary mounts independent host telemetry and distinguishes recording asset size", async () => {
  const summary = await readFile(new URL("../../src/page/Summary.svelte", import.meta.url), "utf8");
  assert.match(summary, /<DiskUsagePanel \/>/);
  assert.match(summary, /录播总大小/);
  assert.doesNotMatch(summary, /invoke.*get_disk_info|disk_info\.total/);
});
