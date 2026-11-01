import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function fixture() {
  const calls = [];
  const context = vm.createContext({});
  const invoker = new vm.SyntheticModule(["invoke"], function () {
    this.setExport("invoke", async (command, args) => {
      calls.push({ command, args });
      return "completed";
    });
  }, { context });
  const mediaInterface = new vm.SyntheticModule(["default_profile", "generateEventId"], function () {
    this.setExport("default_profile", () => ({}));
    this.setExport("generateEventId", () => "media-operation");
  }, { context });

  async function load(name) {
    const source = await readFile(new URL(`../../src/lib/agent/${name}.ts`, import.meta.url), "utf8");
    const { outputText } = ts.transpileModule(source, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
    });
    const module = new vm.SourceTextModule(outputText, { context });
    await module.link((specifier) => {
      if (specifier === "../invoker") return invoker;
      if (specifier === "../interface") return mediaInterface;
      if (specifier === "./messages") return load("messages");
      throw new Error(`Unexpected dependency ${specifier}`);
    });
    return module;
  }

  const tools = await load("tools");
  await tools.evaluate();
  return { invoke: tools.namespace.invokeToolByName, calls };
}

test("stale arbitrary-command confirmations cannot invoke the backend", async () => {
  const f = await fixture();
  await assert.rejects(
    f.invoke("generic_ffmpeg_command", { args: ["-version"] }),
    /not available for frontend confirmation/,
  );
  assert.equal(f.calls.length, 0);
});

test("structured media confirmations retain their operation parameters", async () => {
  const f = await fixture();
  assert.equal(await f.invoke("extract_video_audio", { videoId: 42 }), "completed");
  assert.equal(await f.invoke("merge_videos", {
    videoIds: [42, 43], outputTitle: "Joined", outputNote: "Notes", transition: "fade",
  }), "completed");
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls)), [
    { command: "extract_video_audio", args: { videoId: 42 } },
    { command: "merge_videos", args: {
      videoIds: [42, 43], outputTitle: "Joined", outputNote: "Notes", transition: "fade",
    } },
  ]);
});

test("argument keys stay data and canonical names win throughout nested parameters", async () => {
  const source = `{"clipRangeParams": {
    "__proto__": {"title": "inherited"}, "Title": "explicit", "Constructor": "metadata",
    "roomId": "alias", "room_id": "canonical",
    "ranges": [{"__proto__": {"start": 99}, "Start": 1, "End": 2}, {"Start": 4, "start": 3, "End": 5}]
  }}`;
  for (const args of [source, JSON.parse(source)]) {
    const f = await fixture();
    await f.invoke("clip_range", args);
    const params = f.calls[0].args.params;
    assert.deepEqual(JSON.parse(JSON.stringify(params)), {
      ["__proto__"]: { title: "inherited" },
      title: "explicit",
      constructor: "metadata",
      room_id: "canonical",
      ranges: [{ ["__proto__"]: { start: 99 }, start: 1, end: 2 }, { start: 3, end: 5 }],
    });
    assert.equal(Object.getPrototypeOf(params.ranges[0]), Object.getPrototypeOf(f.calls[0].args));
  }
});
