import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule(globals = {}) {
  const source = await readFile(new URL("../../src/lib/subtitle-ass.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext(globals) });
  await module.link(() => { throw new Error("ASS document generation must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

const styleRow = (code) => code.split("\n").find((line) => line.startsWith("Style: ")).slice(7).split(",");
const replaceFields = (code, fields) => code.replace(/^Style: (.*)$/m, (_, row) => {
  const values = row.split(",");
  for (const [index, value] of Object.entries(fields)) values[Number(index)] = value;
  return `Style: ${values.join(",")}`;
});

test("legacy FFmpeg defaults retain scale, outline, and margins at 1080p", async () => {
  const api = await loadModule();
  const code = api.serializeAssStyle(api.DEFAULT_SUBTITLE_STYLE);
  const values = styleRow(code);
  assert.equal(values.length, 23);
  assert.equal(values[2], "67.5");
  assert.equal(values[16], "7.5");
  assert.deepEqual(values.slice(18, 22), ["2", "100", "100", "75"]);
  const migrated = api.parseAssStyle(code);
  for (const [field, value] of Object.entries(api.DEFAULT_SUBTITLE_STYLE)) assert.equal(migrated[field], value, field);
  assert.equal(api.serializeAssStyle(migrated), code);
});

test("all legacy force_style alignments migrate to their V4+ keypad positions", async () => {
  const api = await loadModule();
  for (const [legacy, ass] of [[1, 1], [2, 2], [3, 3], [5, 7], [6, 8], [7, 9], [9, 4], [10, 5], [11, 6]]) {
    const code = api.serializeAssStyle({ ...api.DEFAULT_SUBTITLE_STYLE, alignment: legacy });
    assert.equal(styleRow(code)[18], String(ass));
    assert.equal(api.parseAssStyle(code).alignment, legacy);
  }
});

test("raw ASS fields and alpha survive easy controls and persistence", async () => {
  const storage = new Map();
  const api = await loadModule({ localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
  } });
  const code = replaceFields(api.serializeAssStyle(api.DEFAULT_SUBTITLE_STYLE), {
    0: "Caption", 1: "思源黑体", 3: "&H80FFCCAA", 4: "&H332211", 5: "&H99000000",
    6: "&H66001122", 7: "-1", 8: "-1", 9: "-1", 10: "-1", 11: "125", 12: "90",
    13: "-0.25", 14: "-12.5", 15: "3", 17: "8.5", 22: "134",
  });
  const edited = api.updateAssFields(api.parseAssStyle(code), { [api.ASS_FIELD.Fontsize]: "90", [api.ASS_FIELD.MarginL]: "125" });
  const values = styleRow(edited.assStyle);
  assert.equal(values[2], "90");
  assert.equal(values[19], "125");
  assert.equal(edited.fontSize, 24);
  assert.equal(edited.marginL, 25);
  for (const index of [0, 1, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 22]) {
    assert.equal(values[index], styleRow(code)[index], `preserve field ${index}`);
  }
  assert.equal(values[4], "&H00332211");
  api.saveSubtitleStyle("room", edited);
  assert.equal(api.loadSubtitleStyle("room").assStyle, edited.assStyle);
  const primary = api.setAssColor(values[3], "#123456");
  const recolored = api.updateAssFields(edited, { [api.ASS_FIELD.PrimaryColour]: primary });
  assert.equal(styleRow(recolored.assStyle)[3], "&H80563412", "picker keeps transparency");
  assert.equal(styleRow(recolored.assStyle)[5], "&H99000000");
  assert.equal(recolored.fontColor, "#123456");
  assert.throws(() => api.updateAssFields(edited, { [api.ASS_FIELD.Fontsize]: "0" }));
});

test("field indices follow the standard Format order and colors convert to CSS", async () => {
  const api = await loadModule();
  assert.deepEqual(api.ASS_STYLE_FORMAT.split(", ").map((field) => api.ASS_FIELD[field]), [...Array(23).keys()]);
  assert.equal(api.cssColor("&H00FFFFFF"), "rgba(255, 255, 255, 1)");
  assert.equal(api.cssColor("&H80563412"), "rgba(18, 52, 86, 0.498)");
  assert.equal(api.cssColor("&HFF000000"), "rgba(0, 0, 0, 0)");
  assert.equal(api.setAssColor("&H332211", "#AABBCC"), "&H00CCBBAA");
});

test("presets are canonical, keep the font and are recognized after applying", async () => {
  const api = await loadModule();
  const font = api.updateAssFields(api.DEFAULT_SUBTITLE_STYLE, { [api.ASS_FIELD.Name]: "Caption", [api.ASS_FIELD.Fontname]: "思源黑体", [api.ASS_FIELD.Encoding]: "134" });
  assert.equal(api.matchSubtitlePreset(font), undefined);
  const ids = new Set();
  for (const preset of api.SUBTITLE_STYLE_PRESETS) {
    ids.add(preset.id);
    assert.equal(preset.fields.split(",").length, 20, preset.id);
    const applied = api.applySubtitlePreset(font, preset);
    const values = styleRow(applied.assStyle);
    assert.deepEqual([values[0], values[1], values[22]], ["Caption", "思源黑体", "134"], preset.id);
    assert.equal(values.slice(2, 22).join(","), preset.fields, `${preset.id} is canonical`);
    assert.equal(api.matchSubtitlePreset(applied)?.id, preset.id);
    assert.equal(api.buildSubtitleAss([{ startTime: 0, endTime: 1, text: "预设" }], applied).includes(`Style: ${values.join(",")}`), true);
  }
  assert.equal(ids.size, api.SUBTITLE_STYLE_PRESETS.length);
});

test("flags pasted as 1 are accepted like libass and kept as written", async () => {
  const api = await loadModule();
  const code = replaceFields(api.serializeAssStyle(api.DEFAULT_SUBTITLE_STYLE), { 7: "1", 8: "-1", 9: "0", 10: "1" });
  assert.deepEqual(styleRow(api.parseAssStyle(code).assStyle).slice(7, 11), ["1", "-1", "0", "1"]);
});

test("standard reordered Format is interpreted and canonicalized", async () => {
  const api = await loadModule();
  const source = api.serializeAssStyle(api.DEFAULT_SUBTITLE_STYLE);
  const fields = api.ASS_STYLE_FORMAT.split(", ");
  const values = styleRow(source);
  const code = `Format: ${fields.reverse().join(", ")}\nStyle: ${values.reverse().join(",")}`;
  assert.equal(api.serializeAssStyle(api.parseAssStyle(code)), source);
});

test("invalid style values and document injection are rejected before rendering", async () => {
  const api = await loadModule();
  const source = api.serializeAssStyle(api.DEFAULT_SUBTITLE_STYLE);
  for (const [field, value] of [
    [0, "*Default"], [1, "font\\name"], [1, "font{\\fnArial}"], [2, "Infinity"],
    [2, "NaN"], [2, "0"], [3, "#ff00"], [3, "&Hzzzzzz"], [7, "5"], [8, "2"],
    [11, "-1"], [15, "2"], [16, "-1"], [18, "10"], [19, "1.5"], [21, "-2"],
  ]) {
    assert.throws(() => api.parseAssStyle(replaceFields(source, { [field]: value })), /./, `${field}: ${value}`);
  }
  for (const code of [
    "", source + "\n[Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,bad",
    source + "\nStyle: " + styleRow(source).join(","), source + "\n[Fonts]",
    source.replace("Name, Fontname", "Name, Name"), source.replace("Arial", "Arial, extra"),
  ]) assert.throws(() => api.parseAssStyle(code), /./);
});

test("legacy localStorage migrates and corrupt/unavailable storage opens with defaults", async () => {
  const storage = new Map();
  const api = await loadModule({ localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
  } });
  storage.set("subtitle_style_old", JSON.stringify({ ...api.DEFAULT_SUBTITLE_STYLE, alignment: 10, fontSize: 30 }));
  assert.equal(styleRow(api.loadSubtitleStyle("old").assStyle)[18], "5");
  assert.equal(styleRow(api.loadSubtitleStyle("old").assStyle)[2], "112.5");
  for (const invalid of ["{", "[]", "null", JSON.stringify({ fontSize: "bad" }), JSON.stringify({ assStyle: "[Events]" })]) {
    storage.set("subtitle_style_bad", invalid);
    assert.equal(api.loadSubtitleStyle("bad").fontSize, 18);
  }
  const denied = await loadModule({ localStorage: { getItem() { throw new Error("denied"); } } });
  assert.equal(denied.loadSubtitleStyle("room").fontSize, 18);
  assert.equal((await loadModule()).loadSubtitleStyle("room").fontSize, 18);
});

test("the shared document has fixed script dimensions, centisecond times and collision layers", async () => {
  const api = await loadModule();
  const doc = api.buildSubtitleAss([
    { startTime: 1, endTime: 2, text: "later read order", layerOrder: 8 },
    { startTime: 1, endTime: 2, text: "first read order", layerOrder: 2 },
    { startTime: 3599.999, endTime: 3601.235, text: "rollover" },
    { startTime: NaN, endTime: 2, text: "invalid" },
    { startTime: 2, endTime: 1, text: "invalid" },
    { startTime: 1.001, endTime: 1.002, text: "below centisecond" },
  ], api.DEFAULT_SUBTITLE_STYLE);
  assert.match(doc, /PlayResX: 1920\nPlayResY: 1080/);
  assert.match(doc, /ScaledBorderAndShadow: yes/);
  const events = doc.split("\n").filter((line) => line.startsWith("Dialogue:"));
  assert.equal(events.length, 3);
  assert.match(events[0], /^Dialogue: 0,0:00:01\.00,0:00:02\.00,Default,,0,0,0,,first read order$/);
  assert.match(events[1], /later read order$/);
  assert.match(events[2], /1:00:00\.00,1:00:01\.24/);
});

test("cue text keeps literal ASS escapes safe and converts line breaks and SRT formatting", async () => {
  const api = await loadModule();
  const doc = api.buildSubtitleAss([{
    startTime: 0, endTime: 1, text: "中文 <b>bold</b><i>italic</i><u>u</u><s>s</s><br>next\r\n{\\pos(1,2)} literal \\N\n[Events]\nDialogue: literal",
  }], api.DEFAULT_SUBTITLE_STYLE);
  assert.match(doc, /中文 \{\\b1\}bold\{\\b0\}\{\\i1\}italic\{\\i0\}\{\\u1\}u\{\\u0\}\{\\s1\}s\{\\s0\}\\Nnext\\N/);
  assert.ok(doc.includes("\\{\\\u2060pos(1,2)\\}"));
  assert.ok(doc.includes("literal \\\u2060N\\N[Events]\\NDialogue: literal"));
  assert.equal(doc.split("\n").filter((line) => line === "[Events]").length, 1);
  assert.equal(doc.split("\n").filter((line) => line.startsWith("Dialogue:")).length, 1);
});

test("SRT font colors restore nested colors and ignore unsafe attributes", async () => {
  const api = await loadModule();
  const doc = api.buildSubtitleAss([{
    startTime: 0, endTime: 1,
    text: '<font color="red">red <font color=\'#0f0\'>green</font> red</font> default <font color="&H123456" face="{\\pos(0,0)}">safe</font> &amp; &#65;',
  }], api.DEFAULT_SUBTITLE_STYLE);
  const event = doc.split("\n").find((line) => line.startsWith("Dialogue:"));
  assert.match(event, /\{\\c&H0000FF&\}red \{\\c&H00FF00&\}green\{\\c&H0000FF&\} red\{\\c\} default safe &amp; &#65;$/);
  assert.ok(!event.includes("pos("));
  assert.ok(!event.includes("<font"));
});
