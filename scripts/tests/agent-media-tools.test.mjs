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
