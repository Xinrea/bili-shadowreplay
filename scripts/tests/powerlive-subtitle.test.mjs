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
