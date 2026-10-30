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
  if (name === "../invoker") {
    return new vm.SyntheticModule(["invoke"], function () {
      this.setExport("invoke", () => { throw new Error("SSR must not probe encoders"); });
    }, { context });
  }
  const exports = await import(name);
  return new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
  }, { context });
});
await component.evaluate();

function selected(html, label) {
  const select = html.match(new RegExp(`<select[^>]*aria-label="${label}"[^>]*>([\\s\\S]*?)</select>`))?.[1];
  assert.ok(select, `Missing ${label}`);
  const option = select.match(/<option\b[^>]*\sselected(?:="")?(?=[\s>])[^>]*>/)?.[0];
  return option?.match(/\bvalue="([^"]*)"/)?.[1];
}

function bitrateMbps(html) {
  const input = html.match(/<input\b[^>]*aria-label="视频码率"[^>]*>/)?.[0];
  assert.ok(input, "Missing 视频码率");
  assert.match(input, /type="number"/);
  assert.match(input, /step="1"/);
  return input.match(/\bvalue="([^"]*)"/)?.[1];
}

test("the shared encoding form starts with HEVC and 10 Mbps while encoders load", () => {
  const { body } = render(component.namespace.default);
  assert.equal(selected(body, "编码格式"), "hevc");
  assert.equal(selected(body, "渲染与编码"), "");
  assert.equal(bitrateMbps(body), "10");
  assert.match(body, /H\.264 \(AVC\)/);
  assert.match(body, /H\.265 \(HEVC\)/);
  assert.match(body, /探测编码器中…/);
  assert.match(body, /<fieldset[^>]* disabled/);
});

test("saved codec, encoder and non-preset bitrate remain visible while encoders load", () => {
  const { body } = render(component.namespace.default, {
    props: { value: {
      ...timeline.namespace.defaultRenderSettings(),
      codec: "h264", priority: "quality", bitrate_kbps: 15000,
      render_backend: "cpu", encoder: "libx264",
    }, disabled: true },
  });
  assert.equal(selected(body, "编码格式"), "h264");
  assert.equal(selected(body, "渲染与编码"), "libx264");
  assert.equal(bitrateMbps(body), "15");
  assert.match(body, /Mbps/);
  assert.match(body, /<fieldset[^>]* disabled/);
});

test("the column layout keeps the same defaults without stacked field spacing", () => {
  const { body } = render(component.namespace.default, { props: { layout: "columns" } });
  assert.equal(selected(body, "编码格式"), "hevc");
  assert.equal(selected(body, "渲染与编码"), "");
  assert.equal(bitrateMbps(body), "10");
  assert.match(body, /<fieldset[^>]*sm:grid-cols-2/);
  assert.doesNotMatch(body, /<fieldset[^>]*space-y-3/);
});

test("settings rows retain selection and let the host provide a single explanation", () => {
  const { body } = render(component.namespace.default, {
    props: {
      layout: "rows",
      showHint: false,
      value: { ...timeline.namespace.defaultRenderSettings(), codec: "h264", priority: "quality", bitrate_kbps: 16000 },
    },
  });
  assert.equal(selected(body, "编码格式"), "h264");
  assert.equal(selected(body, "渲染与编码"), "");
  assert.equal(bitrateMbps(body), "16");
  assert.equal((body.match(/<label class="grid /g) ?? []).length, 3);
  assert.doesNotMatch(body, /仅显示当前导出可用的组合/);
  assert.match(render(component.namespace.default).body, /仅显示当前导出可用的组合/);
});
