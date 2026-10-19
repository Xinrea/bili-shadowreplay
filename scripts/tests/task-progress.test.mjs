import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/task-progress.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("task progress helpers must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

const stage = (fields) => ({ stage: "压制弹幕", processed: 0, ...fields });

test("percent is floored, clamped, and unknown without a total", async () => {
  const { progressPercent } = await loadModule();
  assert.equal(progressPercent(stage({ processed: 12, total: 30 })), 40);
  // Never show completion before the backend reports it.
  assert.equal(progressPercent(stage({ processed: 29.9, total: 30 })), 99);
  assert.equal(progressPercent(stage({ processed: 31, total: 30 })), 100);
  assert.equal(progressPercent(stage({ processed: 12 })), null);
  assert.equal(progressPercent(stage({ processed: 12, total: 0 })), null);
  assert.equal(progressPercent(undefined), null);
});

test("remaining time divides media left by speed", async () => {
  const { remainingSeconds } = await loadModule();
  assert.equal(remainingSeconds(stage({ processed: 10, total: 30, speed: 2 })), 10);
  assert.equal(remainingSeconds(stage({ processed: 31, total: 30, speed: 2 })), 0);
  assert.equal(remainingSeconds(stage({ processed: 10, total: 30 })), null);
  assert.equal(remainingSeconds(stage({ processed: 10, speed: 2 })), null);
});

test("clock and remaining durations read naturally", async () => {
  const { formatClock, formatRemaining } = await loadModule();
  assert.equal(formatClock(75.9), "01:15");
  assert.equal(formatClock(75, true), "00:01:15");
  assert.equal(formatClock(3725), "01:02:05");
  assert.equal(formatRemaining(0.2), "1 秒");
  assert.equal(formatRemaining(59.1), "1 分钟");
  assert.equal(formatRemaining(61), "2 分钟");
  assert.equal(formatRemaining(3599), "1 小时");
  assert.equal(formatRemaining(3601), "1 小时 1 分");
  assert.equal(formatRemaining(7200), "2 小时");
});

test("tasks are described from their metadata", async () => {
  const { describeTask, taskTypeName } = await loadModule();
  const clip = {
    params: {
      title: "晚间直播",
      ranges: [
        { start: 10, end: 40 },
        { start: 100, end: 160 },
        { start: 200, end: 260, activated: false },
      ],
    },
  };
  assert.equal(describeTask({ metadata: JSON.stringify(clip) }), "晚间直播 · 2 个选区 · 01:30");
  assert.equal(describeTask({ metadata: JSON.stringify({ clip_title: "高光" }) }), "高光");
  assert.equal(
    describeTask({ metadata: JSON.stringify({ clip_title: "高光", start_time: 10, end_time: 75 }) }),
    "高光 · 01:05",
  );
  assert.equal(describeTask({ metadata: JSON.stringify({ video_id: 7, profile: { title: "投稿标题" } }) }), "投稿标题");
  assert.equal(describeTask({ metadata: JSON.stringify({ video_id: 7 }) }), "素材 #7");
  assert.equal(describeTask({ metadata: JSON.stringify({ project_id: 3, title: "精彩集锦" }) }), "精彩集锦");
  assert.equal(describeTask({ metadata: JSON.stringify({ project_id: 3 }) }), "切片 #3");
  assert.equal(describeTask({ metadata: "not json" }), "");
  assert.equal(taskTypeName("clip_range"), "素材生成");
  assert.equal(taskTypeName("export_project"), "切片导出");
  assert.equal(taskTypeName("upload"), "投稿");
  assert.equal(taskTypeName("render_material_danmu"), "素材压制弹幕");
  assert.equal(taskTypeName("unknown_kind"), "unknown_kind");
});

test("only queued task types offer cancellation", async () => {
  const { isCancellableTask } = await loadModule();
  for (const type of ["clip_range", "generate_materials", "export_project", "render_material_danmu", "clip_video", "encode_video_subtitle", "generate_whole_clip", "generate_archive_summary"]) {
    assert.equal(isCancellableTask(type), true, type);
  }
  for (const type of ["generate_video_subtitle", "upload", "upload_procedure", "upload_project"]) {
    assert.equal(isCancellableTask(type), false, type);
  }
});
