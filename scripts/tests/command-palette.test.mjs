import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/command-palette.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("command palette helpers must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

const command = (id, group, title, fields = {}) => ({ id, group, title, ...fields });

const commands = [
  command("page:设置", "page", "设置", { keywords: ["settings", "preferences"] }),
  command("page:切片", "page", "切片", { keywords: ["clips"] }),
  command("room:add", "action", "添加直播间", { detail: "直播间", keywords: ["add room"] }),
  command("clip:import", "action", "导入视频", { detail: "切片", keywords: ["import video"] }),
  command("setting:cache-path", "setting", "缓存路径", { detail: "存储设置", keywords: ["cache"] }),
  command("setting:output-path", "setting", "切片保存路径", { detail: "存储设置", keywords: ["output"] }),
  command("setting:notify-clip", "setting", "切片完成通知", { detail: "通知设置" }),
];

const ids = (sections) => sections.flatMap((section) => section.commands.map((entry) => entry.id));
// Values built inside the vm context have their own prototypes.
const plain = (value) => JSON.parse(JSON.stringify(value));

test("an empty query groups commands, recent ones first and not repeated", async () => {
  const { paletteSections } = await loadModule();
  const sections = paletteSections(commands, "  ", ["setting:cache-path", "missing", "room:add"]);
  assert.deepEqual(
    plain(sections.map((section) => section.label)),
    ["最近使用", "页面", "功能", "设置项"],
  );
  assert.deepEqual(plain(ids(sections.slice(0, 1))), ["setting:cache-path", "room:add"]);
  assert.deepEqual(plain(ids(sections)), [
    "setting:cache-path",
    "room:add",
    "page:设置",
    "page:切片",
    "clip:import",
    "setting:output-path",
    "setting:notify-clip",
  ]);
});

test("unavailable commands are left out, including from recent ones", async () => {
  const { paletteSections } = await loadModule();
  const hidden = [...commands, command("ai:clear", "action", "清空助手对话", { available: () => false })];
  assert.ok(!ids(paletteSections(hidden, "", ["ai:clear"])).includes("ai:clear"));
  assert.deepEqual(plain(ids(paletteSections(hidden, "清空"))), []);
});

test("a query ranks titles above keywords and details", async () => {
  const { paletteSections } = await loadModule();
  const sections = paletteSections(commands, "切片");
  assert.equal(sections.length, 1);
  assert.equal(sections[0].label, undefined);
  // Exact title, then title prefixes, then the action whose page is 切片.
  assert.deepEqual(plain(ids(sections)), [
    "page:切片",
    "setting:output-path",
    "setting:notify-clip",
    "clip:import",
  ]);
});

test("keywords match case-insensitively", async () => {
  const { paletteSections } = await loadModule();
  assert.deepEqual(plain(ids(paletteSections(commands, "Settings"))), ["page:设置"]);
  assert.deepEqual(plain(ids(paletteSections(commands, "CACHE"))), ["setting:cache-path"]);
});

test("characters may be spread out, within limits", async () => {
  const { paletteSections } = await loadModule();
  assert.deepEqual(plain(ids(paletteSections(commands, "切路"))), ["setting:output-path"]);
  assert.deepEqual(plain(ids(paletteSections(commands, "adrm"))), ["room:add"]);
  const long = [command("long", "action", "a" + "x".repeat(20) + "b")];
  assert.deepEqual(plain(ids(paletteSections(long, "ab"))), []);
});

test("every term of a query has to match", async () => {
  const { paletteSections } = await loadModule();
  assert.deepEqual(plain(ids(paletteSections(commands, "存储 缓存"))), ["setting:cache-path"]);
  assert.deepEqual(plain(ids(paletteSections(commands, "存储 不存在"))), []);
});

test("recent commands move to the front and are capped", async () => {
  const { rememberRecent, RECENT_LIMIT } = await loadModule();
  assert.deepEqual(plain(rememberRecent(["a", "b", "c"], "b")), ["b", "a", "c"]);
  const full = Array.from({ length: RECENT_LIMIT }, (_, index) => `id${index}`);
  const next = rememberRecent(full, "new");
  assert.equal(next.length, RECENT_LIMIT);
  assert.equal(next[0], "new");
  assert.ok(!next.includes(`id${RECENT_LIMIT - 1}`));
});

test("the shortcut is Ctrl or ⌘ with Shift and P on any layout", async () => {
  const { isPaletteShortcut } = await loadModule();
  const key = (fields) => ({ ctrlKey: false, metaKey: false, shiftKey: true, altKey: false, key: "P", code: "KeyP", ...fields });
  assert.ok(isPaletteShortcut(key({ ctrlKey: true })));
  assert.ok(isPaletteShortcut(key({ metaKey: true })));
  // Another layout puts a different character on the physical P key.
  assert.ok(isPaletteShortcut(key({ ctrlKey: true, key: "З" })));
  assert.ok(!isPaletteShortcut(key({})));
  assert.ok(!isPaletteShortcut(key({ ctrlKey: true, shiftKey: false, key: "p" })));
  assert.ok(!isPaletteShortcut(key({ ctrlKey: true, altKey: true })));
  assert.ok(!isPaletteShortcut(key({ ctrlKey: true, key: "O", code: "KeyO" })));
});
