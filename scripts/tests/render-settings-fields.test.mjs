import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";
import ts from "typescript";

// Render the actual shared form, including Svelte's select bindings, without
// introducing a browser dependency into the unit test suite.
const context = vm.createContext({});
const timelineSource = await readFile(new URL("../../src/lib/project-timeline.ts", import.meta.url), "utf8");
const timeline = new vm.SourceTextModule(ts.transpileModule(timelineSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText, { context });
await timeline.link(() => { throw new Error("Unexpected timeline dependency"); });
await timeline.evaluate();
const source = await readFile(new URL("../../src/lib/components/RenderSettingsFields.svelte", import.meta.url), "utf8");
const component = new vm.SourceTextModule(compile(source, { generate: "server" }).js.code, { context });
await component.link(async (name) => {
  if (name === "../project-timeline") return timeline;
  const exports = await import(name);
  return new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
  }, { context });
});
await component.evaluate();

function selected(html, label) {
  const select = html.match(new RegExp(`<select[^>]*aria-label="${label}"[^>]*>([\\s\\S]*?)</select>`))?.[1];
  assert.ok(select, `Missing ${label}`);
  return select.match(/<option value="([^"]*)" selected(?:="")?>/)?.[1];
}

test("the shared encoding form defaults to HEVC, speed and 10 Mbps", () => {
  const { body } = render(component.namespace.default);
  assert.equal(selected(body, "编码格式"), "hevc");
  assert.equal(selected(body, "编码偏好"), "speed");
  assert.equal(selected(body, "视频码率"), "10000");
  assert.match(body, /H\.264 \(AVC\)/);
  assert.match(body, /H\.265 \(HEVC\)/);
  assert.match(body, /画质优先/);
  assert.match(body, /速度优先/);
});

test("all three independent settings and non-preset saved bitrates are displayed", () => {
  const { body } = render(component.namespace.default, {
    props: { value: { codec: "h264", priority: "quality", bitrate_kbps: 15000 }, disabled: true },
  });
  assert.equal(selected(body, "编码格式"), "h264");
  assert.equal(selected(body, "编码偏好"), "quality");
  assert.equal(selected(body, "视频码率"), "15000");
  assert.match(body, /15 Mbps/);
  assert.match(body, /<fieldset[^>]* disabled/);
});

test("the column layout keeps the same defaults without stacked field spacing", () => {
  const { body } = render(component.namespace.default, { props: { layout: "columns" } });
  assert.equal(selected(body, "编码格式"), "hevc");
  assert.equal(selected(body, "编码偏好"), "speed");
  assert.equal(selected(body, "视频码率"), "10000");
  assert.match(body, /<fieldset[^>]*sm:grid-cols-3/);
  assert.doesNotMatch(body, /<fieldset[^>]*space-y-3/);
});

test("settings rows retain selection and let the host provide a single explanation", () => {
  const { body } = render(component.namespace.default, {
    props: {
      layout: "rows",
      showHint: false,
      value: { codec: "h264", priority: "quality", bitrate_kbps: 16000 },
    },
  });
  assert.equal(selected(body, "编码格式"), "h264");
  assert.equal(selected(body, "编码偏好"), "quality");
  assert.equal(selected(body, "视频码率"), "16000");
  assert.equal((body.match(/<label class="grid /g) ?? []).length, 3);
  assert.doesNotMatch(body, /实际效果取决于编码器/);
  assert.match(render(component.namespace.default).body, /实际效果取决于编码器/);
});
