import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/platform.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("platform helpers must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

test("platform labels fall back to the id, and to 未知 when missing", async () => {
  const { platformLabel } = await loadModule();
  assert.equal(platformLabel("bilibili"), "B站");
  assert.equal(platformLabel("Douyu"), "斗鱼");
  assert.equal(platformLabel("youtube"), "YouTube");
  assert.equal(platformLabel("someplace"), "someplace");
  assert.equal(platformLabel(undefined), "未知");
  assert.equal(platformLabel(""), "未知");
});

test("unknown platform labels cannot resolve inherited object properties", async () => {
  const { platformLabel } = await loadModule();
  for (const platform of ["constructor", "Constructor", "__proto__", "__PROTO__"]) {
    assert.equal(platformLabel(platform), platform);
  }
});

test("live room urls follow each platform's room page", async () => {
  const { liveRoomUrl } = await loadModule();
  assert.equal(liveRoomUrl("bilibili", "167537"), "https://live.bilibili.com/167537");
  assert.equal(liveRoomUrl("douyin", "200525029536"), "https://live.douyin.com/200525029536");
  assert.equal(liveRoomUrl("douyu", "9999"), "https://www.douyu.com/9999");
  assert.equal(liveRoomUrl("huya", "660000"), "https://www.huya.com/660000");
  assert.equal(liveRoomUrl("kuaishou", "abc"), "https://live.kuaishou.com/u/abc");
  assert.equal(liveRoomUrl("tiktok", "@someone"), "https://www.tiktok.com/@someone/live");
  assert.equal(liveRoomUrl("twitch", "streamer"), "https://www.twitch.tv/streamer");
  assert.equal(liveRoomUrl("BiliBili", "1"), "https://live.bilibili.com/1");
});

test("youtube room ids map to channel, handle, legacy and video pages", async () => {
  const { liveRoomUrl } = await loadModule();
  assert.equal(liveRoomUrl("youtube", "legacy-c-Name"), "https://www.youtube.com/c/Name/live");
  assert.equal(liveRoomUrl("youtube", "legacy-user-Name"), "https://www.youtube.com/user/Name/live");
  assert.equal(liveRoomUrl("youtube", "dQw4w9WgXcQ"), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
  assert.equal(
    liveRoomUrl("youtube", "UCxxxxxxxxxxxxxxxxxxxxxx"),
    "https://www.youtube.com/channel/UCxxxxxxxxxxxxxxxxxxxxxx/live",
  );
  assert.equal(liveRoomUrl("youtube", "handle"), "https://www.youtube.com/@handle/live");
  assert.equal(liveRoomUrl("youtube", "@handle"), "https://www.youtube.com/@handle/live");
});

test("url room ids pass through and unknown platforms have no page", async () => {
  const { liveRoomUrl } = await loadModule();
  const url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
  assert.equal(liveRoomUrl("youtube", url), url);
  assert.equal(liveRoomUrl("imported", "123"), null);
  assert.equal(liveRoomUrl(undefined, "123"), null);
});
