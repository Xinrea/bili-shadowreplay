import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/powerlive-subtitle.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("PowerLive subtitle helpers must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

test("hotwords split on the separators PowerLive accepts", async () => {
  const { splitHotwords } = await loadModule();
  assert.deepEqual([...splitHotwords("黄泉,砂金，遗器、芙宁娜;原神；Honkai   Star Rail\n\n舰长")], [
    "黄泉", "砂金", "遗器", "芙宁娜", "原神", "Honkai Star Rail", "舰长",
  ]);
  assert.deepEqual([...splitHotwords(" , ，")], []);
});

test("room dictionaries stay isolated by platform and room and support older configs", async () => {
  const { roomHotwords } = await loadModule();
  const dictionaries = {
    bilibili: { "1": ["主播甲"], "2": ["主播乙"] },
    douyu: { "1": ["主播丙"] },
  };
  assert.deepEqual([...roomHotwords(dictionaries, { platform: "bilibili", roomId: "1" })], ["主播甲"]);
  assert.deepEqual([...roomHotwords(dictionaries, { platform: "douyu", roomId: "1" })], ["主播丙"]);
  assert.deepEqual([...roomHotwords(dictionaries, { platform: "bilibili", roomId: "2" })], ["主播乙"]);
  assert.deepEqual([...roomHotwords(dictionaries, { platform: "bilibili", roomId: "3" })], []);
  assert.deepEqual([...roomHotwords(undefined, { platform: "bilibili", roomId: "1" })], []);
});

test("recognition merges room hotwords first without modifying either dictionary", async () => {
  const { mergeHotwords, MAX_HOTWORDS } = await loadModule();
  const room = [" 主播甲 ", "Honkai   Star Rail", "共用词", ""];
  const global = ["全局词", "共用词", "Honkai Star Rail"];
  assert.deepEqual([...mergeHotwords(room, global)], ["主播甲", "Honkai Star Rail", "共用词", "全局词"]);
  assert.equal(room[0], " 主播甲 ");
  assert.equal(global.length, 3);
  assert.deepEqual([...mergeHotwords([], global)], global);
  const many = Array.from({ length: MAX_HOTWORDS }, (_, i) => `词${i}`);
  const merged = mergeHotwords(["主播甲"], many);
  assert.equal(merged.length, MAX_HOTWORDS + 1); // The UI can warn before the request is truncated.
  assert.equal(merged.slice(0, MAX_HOTWORDS)[0], "主播甲");
  assert.ok(!merged.slice(0, MAX_HOTWORDS).includes(many[MAX_HOTWORDS - 1]));
});

test("hotwords PowerLive would drop are refused with a reason", async () => {
  const { hotwordProblem } = await loadModule();
  assert.equal(hotwordProblem("司徒钰芊"), null);
  assert.equal(hotwordProblem("崩坏星穹铁道"), null);
  assert.equal(hotwordProblem("Honkai Star Rail"), null);
  assert.equal(hotwordProblem("一二三四五六七八九十一二三四五"), null);
  assert.match(hotwordProblem("一二三四五六七八九十一二三四五六"), /15/);
  assert.match(hotwordProblem("你好！"), /标点/);
  assert.match(hotwordProblem("don't"), /标点/);
  assert.match(hotwordProblem("a b c d e f g h"), /7/);
  assert.match(hotwordProblem("x".repeat(65)), /64/);
});

test("the context names the streamer, the stream titles and the clip", async () => {
  const { buildSubtitleContext } = await loadModule();
  assert.equal(
    buildSubtitleContext({
      platform: "B站",
      streamer: "司徒钰芊",
      liveTitles: ["星穹铁道新版本", "", "星穹铁道新版本", "原神深渊"],
      clipTitle: "砂金抽卡",
    }),
    "B站主播司徒钰芊的直播，直播标题：「星穹铁道新版本」「原神深渊」，切片标题：「砂金抽卡」。",
  );
  assert.equal(
    buildSubtitleContext({ platform: "", streamer: "", liveTitles: [], clipTitle: "原神深渊" }),
    "切片标题：「原神深渊」。",
  );
  assert.equal(
    buildSubtitleContext({ platform: "虎牙", streamer: "", liveTitles: ["深夜电台"], clipTitle: "" }),
    "虎牙直播录像，直播标题：「深夜电台」。",
  );
  assert.equal(buildSubtitleContext({ platform: "", streamer: " ", liveTitles: [""], clipTitle: "" }), "");
  // A clip named after its stream says nothing new.
  assert.equal(
    buildSubtitleContext({ platform: "抖音", streamer: "A", liveTitles: ["标题"], clipTitle: "标题" }),
    "抖音主播A的直播，直播标题：「标题」。",
  );
});

test("the context keeps whole titles within PowerLive's limit", async () => {
  const { buildSubtitleContext, MAX_CONTEXT_CHARS } = await loadModule();
  const titles = Array.from({ length: 40 }, (_, index) => `第${index}场直播的超长标题`);
  const context = buildSubtitleContext({ platform: "B站", streamer: "主播", liveTitles: titles, clipTitle: "切片" });
  assert.ok([...context].length <= MAX_CONTEXT_CHARS);
  assert.ok(context.endsWith("」。") || context.endsWith("」，切片标题：「切片」。"));
  assert.ok(!context.includes("第39场"));
});

// Run the component's actual save handler, with only its bindings and I/O mocked.
async function hotwordEditor(room, invoke) {
  const source = await readFile(new URL("../../src/lib/components/PowerLiveHotwords.svelte", import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("Hotwords.ts", script, ts.ScriptTarget.Latest, true);
  const handler = parsed.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "save");
  const { outputText } = ts.transpileModule(handler.getText(parsed), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  });
  const { roomHotwords } = await loadModule();
  const state = { room, invoke, roomHotwords, hotwords: [], problem: "", saves: 0, saving: Promise.resolve() };
  vm.runInContext(outputText, vm.createContext(state));
  return state;
}

test("each editor saves only its own dictionary and preserves rapid edit order", async () => {
  for (const room of [undefined, { platform: "bilibili", roomId: "1" }]) {
    const calls = [];
    const editor = await hotwordEditor(room, async (command, args) => {
      assert.equal(command, "update_powerlive_hotwords");
      calls.push(JSON.parse(JSON.stringify(args)));
      return args.hotwords;
    });
    editor.save(["主播甲"]);
    editor.save(["主播甲", "角色名"]);
    editor.save(["角色名"]);
    await editor.saving;
    assert.deepEqual(calls, [
      { hotwords: ["主播甲"], ...room },
      { hotwords: ["主播甲", "角色名"], ...room },
      { hotwords: ["角色名"], ...room },
    ]);
    assert.deepEqual([...editor.hotwords], ["角色名"]);
  }
});

test("a failed room save restores that room, not the global dictionary", async () => {
  const editor = await hotwordEditor({ platform: "bilibili", roomId: "1" }, async (command) => {
    if (command === "update_powerlive_hotwords") throw new Error("disk full");
    assert.equal(command, "get_config");
    return { powerlive_hotwords: ["全局词"], powerlive_room_hotwords: { bilibili: { "1": ["已保存主播"] } } };
  });
  editor.save(["新主播"]);
  await editor.saving;
  assert.deepEqual([...editor.hotwords], ["已保存主播"]);
  assert.match(editor.problem, /保存热词失败/);
});

test("the AI dialog loads both dictionaries and sends their current merged values", async () => {
  const source = await readFile(new URL("../../src/lib/components/project/ProjectEditor.svelte", import.meta.url), "utf8");
  const parsed = ts.createSourceFile("ProjectEditor.ts", source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1], ts.ScriptTarget.Latest, true);
  const handlers = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && ["openAiModal", "generateSubtitles"].includes(node.name?.text));
  assert.equal(handlers.length, 2);
  const { outputText } = ts.transpileModule(handlers.map((node) => node.getText(parsed)).join("\n"), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  });
  const helpers = await loadModule();
  const requests = [];
  const state = {
    ...Object.fromEntries(Object.entries(helpers)),
    project: { platform: "bilibili" }, roomId: "1", TAURI_ENV: false,
    aiGlobalHotwords: [], aiRoomHotwords: [], aiRoom: undefined, aiSentenceLength: 10,
    aiOverwrite: false, aiRegenerate: false, aiContext: "", aiPowerLive: false, aiRunning: false, showAiModal: false,
    timeline: { clips: [{ id: "clip", material_id: 1, start: 0, end: 10 }], subtitles: [] },
    fillSubtitleContext: async () => {},
    invoke: async (command, args) => {
      if (command === "get_config") return {
        subtitle_generator_type: "powerlive", powerlive_hotwords: ["全局词", "共用词"],
        powerlive_room_hotwords: { bilibili: { "1": ["主播甲", "共用词"] }, douyu: { "1": ["主播乙"] } },
      };
      if (command === "get_video_subtitle") return "";
      assert.equal(command, "generate_video_subtitle");
      requests.push(JSON.parse(JSON.stringify(args)));
      return "";
    },
    generateEventId: () => "subtitle-1", announceTask() {}, openTaskCenter() {}, settleTask() {},
    parseSrt: () => [], setClipSubtitles: (timeline) => timeline, commit() {},
    alert(message) { assert.fail(message); }, console,
  };
  Object.defineProperty(state, "aiMergedHotwords", { get: () => helpers.mergeHotwords(state.aiRoomHotwords, state.aiGlobalHotwords) });
  vm.runInContext(outputText, vm.createContext(state));
  await state.openAiModal();
  assert.equal(state.showAiModal, true);
  assert.equal(state.aiPowerLive, true);
  assert.deepEqual([...state.aiRoomHotwords], ["主播甲", "共用词"]);
  assert.deepEqual([...state.aiGlobalHotwords], ["全局词", "共用词"]);
  // Even if automatic saves are pending, recognize with what the user sees.
  state.aiRoomHotwords.push("新角色");
  await state.generateSubtitles();
  assert.deepEqual(requests[0].powerlive.hotwords, ["主播甲", "共用词", "新角色", "全局词"]);
  assert.equal(state.aiRunning, false);
  state.project = { platform: "douyu" };
  await state.openAiModal();
  assert.deepEqual([...state.aiRoomHotwords], ["主播乙"]);
  state.roomId = "";
  await state.openAiModal();
  assert.equal(state.aiRoom, undefined);
  assert.deepEqual([...state.aiRoomHotwords], []);
});

test("line lengths become the nearest whole one PowerLive accepts", async () => {
  const { clampSentenceLength } = await loadModule();
  assert.equal(clampSentenceLength(12), 12);
  assert.equal(clampSentenceLength(12.6), 13);
  assert.equal(clampSentenceLength(1), 4);
  assert.equal(clampSentenceLength(99), 50);
  // An emptied number field binds null or undefined.
  assert.equal(clampSentenceLength(null), 10);
  assert.equal(clampSentenceLength(undefined), 10);
  assert.equal(clampSentenceLength(Number.NaN), 10);
});
