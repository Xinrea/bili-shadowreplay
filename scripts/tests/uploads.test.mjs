import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

const DEFAULT_PROFILE = { videos: [], cover: "", title: "", tid: 27, tag: "", desc: "", dynamic: "", copyright: 1 };

async function transpile(path) {
  const source = await readFile(new URL(`../../src/lib/${path}`, import.meta.url), "utf8");
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}

/** Load uploads.ts with real timeline and ASS helpers and stubbed app modules. */
async function loadModule(storage = {}, invoke = async () => {}) {
  const context = vm.createContext({
    crypto: globalThis.crypto,
    localStorage: { getItem: (key) => storage[key] ?? null, setItem: (key, value) => (storage[key] = value) },
  });
  const stub = (exports) =>
    new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
  const modules = new Map();
  async function link(specifier) {
    if (specifier === "svelte/store") return stub({ writable: (value) => ({ set() {}, subscribe() {}, value }) });
    if (specifier === "./interface") return stub({ default_profile: () => structuredClone(DEFAULT_PROFILE) });
    if (specifier === "./invoker") return stub({ invoke });
    const path = `${specifier.replace("./", "")}.ts`;
    if (!modules.has(path)) modules.set(path, new vm.SourceTextModule(await transpile(path), { context }));
    return modules.get(path);
  }
  const module = new vm.SourceTextModule(await transpile("uploads.ts"), { context });
  await module.link(link);
  await module.evaluate();
  return module.namespace;
}

// Values created inside the VM context have its own prototypes.
const plain = (value) => JSON.parse(JSON.stringify(value));

test("a new draft copies the room's last form under the source's title", async () => {
  const template = { ...DEFAULT_PROFILE, title: "上次的标题", tid: 171, tag: "录播", videos: [{ cid: 1 }] };
  const api = await loadModule({ "profile-1": JSON.stringify(template) });
  assert.deepEqual(plain(api.draftProfile("1", "新切片")), { ...template, title: "新切片", videos: [] });
  assert.deepEqual(plain(api.draftProfile("2", "其他房间")), { ...DEFAULT_PROFILE, title: "其他房间" });
  assert.deepEqual(plain(api.draftProfile("", "视频文件")), { ...DEFAULT_PROFILE, title: "视频文件" });
});

test("a project's own saved form wins and keeps its title", async () => {
  const api = await loadModule({ "profile-1": JSON.stringify({ ...DEFAULT_PROFILE, tid: 171 }) });
  // Projects posted before uploads existed saved only part of the form.
  const legacy = JSON.stringify({ title: "已投稿", desc: "简介", tag: "a,b", tid: 21 });
  assert.deepEqual(plain(api.draftProfile("1", "切片标题", legacy)), {
    ...DEFAULT_PROFILE,
    title: "已投稿",
    desc: "简介",
    tag: "a,b",
    tid: 21,
  });
  assert.equal(api.draftProfile("1", "切片标题", JSON.stringify({ desc: "x" })).title, "切片标题");
});

test("stored forms are completed and broken ones fall back to defaults", async () => {
  const api = await loadModule();
  assert.deepEqual(plain(api.parseProfile('{"title":"t","videos":[{"cid":1}]}')), { ...DEFAULT_PROFILE, title: "t" });
  assert.deepEqual(plain(api.parseProfile("")), DEFAULT_PROFILE);
  assert.deepEqual(plain(api.parseProfile("not json")), DEFAULT_PROFILE);
  assert.equal(api.uploadTitle({ profile: "" }), "未命名投稿");
});

test("only projects with visible subtitles carry an ASS document", async () => {
  const api = await loadModule();
  const clips = [{ id: "c1", material_id: 1, start: 10, end: 20 }];
  const project = (subtitles) => ({ room_id: "1", timeline: JSON.stringify({ version: 1, clips, subtitles, danmu: false }) });
  assert.equal(api.projectSubtitleAss(project([]), 30), null);
  // Outside the clip's range, so not exported.
  assert.equal(api.projectSubtitleAss(project([{ id: "s1", clip_id: "c1", start: 30, end: 32, text: "hidden" }]), 30), null);
  // From 2 s to 4 s, between the frames before them at 30 FPS, as the editor writes them.
  const ass = api.projectSubtitleAss(project([{ id: "s1", clip_id: "c1", start: 12, end: 14, text: "你好" }]), 30);
  assert.match(ass, /^Dialogue: 0,0:00:01\.98,0:00:03\.98,.*你好$/m);
});

test("a source's unposted draft is reused, and a new one started once it is posted", async () => {
  const calls = [];
  const api = await loadModule({}, async (command, args) => {
    calls.push({ command, args: plain(args) });
    return { id: 9 };
  });
  const project = { id: 3, room_id: "", title: "切片", profile: "" };
  const upload = (id, status, source = "project", source_id = 3) => ({ id, status, source, source_id });
  const pick = { source: "project", item: project };

  assert.equal((await api.draftUpload(pick, [upload(1, "posted"), upload(2, "draft")])).id, 2);
  assert.equal((await api.draftUpload(pick, [upload(1, "failed")])).id, 1);
  assert.equal(calls.length, 0);

  // Posted, running, or of another source: none is reused.
  const others = [
    upload(1, "posted"),
    upload(2, "posting"),
    upload(3, "exporting"),
    upload(4, "draft", "material"),
    upload(5, "draft", "project", 8),
  ];
  assert.equal((await api.draftUpload(pick, others)).id, 9);
  assert.deepEqual(calls, [
    {
      command: "create_upload",
      args: { source: "project", sourceId: 3, file: "", profile: { ...DEFAULT_PROFILE, title: "切片" } },
    },
  ]);

  // An untitled material is named after its file.
  calls.length = 0;
  await api.draftUpload({ source: "material", item: { id: 4, room_id: "", title: "", file: "a/b/录播.mp4" } }, []);
  assert.equal(calls[0].args.profile.title, "录播");
});

test("the source description says how the video is found", async () => {
  const api = await loadModule();
  assert.match(api.sourceDescription({ source: "material", source_state: "material" }), /素材的视频文件/);
  assert.match(api.sourceDescription({ source: "material", source_state: "render", danmu: true }), /压制弹幕后生成视频/);
  assert.match(
    api.sourceDescription({ source: "material", source_state: "material", danmu_embedded: true }),
    /弹幕已压制在画面中/,
  );
  assert.match(
    api.sourceDescription({ source: "project", source_state: "render", danmu: false, danmu_embedded: true }),
    /素材画面中已压制弹幕/,
  );
  assert.match(api.sourceDescription({ source: "project", source_state: "material" }), /直接使用素材文件/);
  assert.match(api.sourceDescription({ source: "project", source_state: "render", danmu: true }), /生成视频，压制弹幕/);
  assert.match(api.sourceDescription({ source: "project", source_state: "render", danmu: false }), /不压制弹幕/);
  assert.match(api.sourceDescription({ source: "file", source_state: "file" }), /选定的视频文件/);
  assert.match(api.sourceDescription({ source: "file", source_state: "missing" }), /不存在/);
  assert.match(api.sourceDescription({ source: "project", source_state: "missing" }), /已被删除/);
});
