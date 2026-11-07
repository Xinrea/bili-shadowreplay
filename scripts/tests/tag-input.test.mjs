import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/TagInput.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("TagInput.ts", script, ts.ScriptTarget.Latest, true);
// Run the component's actual handlers and default props, without a DOM.
const code = ts.transpileModule(parsed.statements.filter((node) => !ts.isImportDeclaration(node))
  .map((node) => node.getText(parsed)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const plain = (value) => JSON.parse(JSON.stringify(value));

function fixture(props = {}) {
  const changes = [];
  const context = vm.createContext({
    $props: () => ({
      value: [],
      onChange(next) {
        changes.push(plain(next));
        context.next = next;
        vm.runInContext("value = next", context);
      },
      ...props,
    }),
    $state: (value) => value,
    $bindable: (value) => value,
  });
  vm.runInContext(code, context);
  return {
    changes,
    run: (expression) => vm.runInContext(expression, context),
    get: (expression) => plain(vm.runInContext(expression, context)),
    type(text) { context.text = text; vm.runInContext("draft = text", context); },
    key(key, isComposing = false) {
      let prevented = false;
      context.onkeydown({ key, isComposing, preventDefault() { prevented = true; } });
      return prevented;
    },
  };
}

test("tags commit on Enter, deduplicate, and accept punctuation unlike hotwords", () => {
  const f = fixture();
  f.type("  游戏，直播,游戏\nC++\r\n ");
  assert.equal(f.key("Enter"), true);
  assert.deepEqual(f.changes, [["游戏", "直播", "C++"]]);
  assert.equal(f.get("draft"), "");
  f.type("直播");
  f.key(",");
  assert.equal(f.changes.length, 1);
});

test("Chinese composition is not committed and comma shortcuts still work", () => {
  const f = fixture();
  f.type("游戏");
  assert.equal(f.key("Enter", true), false);
  assert.equal(f.key("，", true), false);
  assert.equal(f.changes.length, 0);
  assert.equal(f.get("draft"), "游戏");
  assert.equal(f.key("，"), true);
  assert.deepEqual(f.changes, [["游戏"]]);
});

test("removal and empty-draft Backspace notify the owner, including the last tag", () => {
  const f = fixture({ value: ["游戏", "直播"] });
  f.type("未提交");
  f.key("Backspace");
  assert.equal(f.changes.length, 0);
  f.run('remove("游戏")');
  f.type("");
  f.key("Backspace");
  f.key("Backspace");
  assert.deepEqual(f.changes, [["直播"], []]);
});

test("disabled editors neither commit nor remove", () => {
  const f = fixture({ value: ["游戏"], disabled: true });
  f.type("直播");
  f.run("commit()");
  f.key("Enter");
  f.run('remove("游戏")');
  f.type("");
  f.key("Backspace");
  assert.deepEqual(f.changes, []);
});

test("caller-specific validation, splitting, and limits preserve refused drafts", () => {
  const f = fixture({
    label: "热词",
    max: 2,
    value: ["已有"],
    split: (text) => text.split(";"),
    validate: (word) => word.includes("!") ? "不能包含标点" : null,
  });
  f.type("拒绝!;新增;超额");
  f.run("commit()");
  assert.deepEqual(f.changes, [["已有", "新增"]]);
  assert.equal(f.get("draft"), "拒绝!，超额");
  assert.equal(f.get("problem"), "「拒绝!」不能包含标点");
  f.type("超额");
  f.run("commit()");
  assert.equal(f.get("problem"), "最多 2 个热词");
});

test("upload tag changes keep the persisted string format and schedule autosave", async () => {
  const upload = await readFile(new URL("../../src/lib/components/upload/UploadEditor.svelte", import.meta.url), "utf8");
  const script = upload.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("UploadEditor.ts", script, ts.ScriptTarget.Latest, true);
  const handler = parsed.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "setTags");
  const code = ts.transpileModule(handler.getText(parsed), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  let saves = 0;
  const state = { profile: { tag: "旧标签" }, scheduleSave: () => saves++ };
  vm.createContext(state);
  vm.runInContext(code, state);
  state.setTags(["游戏", "直播"]);
  assert.equal(state.profile.tag, "游戏,直播");
  state.setTags([]);
  assert.equal(state.profile.tag, "");
  assert.equal(saves, 2);
  assert.match(upload, /onChange=\{setTags\}/);
  assert.match(source, /onblur=\{\(\) => draft.trim\(\) && commit\(\)\}/);
});
