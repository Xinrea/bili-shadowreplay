import type { SubtitleStyle } from "./interface";

export interface SubtitleCue {
  startTime: number;
  endTime: number;
  text: string;
  layerOrder?: number;
}

// Existing settings were FFmpeg force_style values for its 384 × 288 SRT
// script. Keep those units in storage and the public model during migration.
const SCALE_X = 1920 / 384;
const SCALE_Y = 1080 / 288;
const LEGACY_TO_ASS: Record<number, number> = {
  1: 1, 2: 2, 3: 3, 5: 7, 6: 8, 7: 9, 9: 4, 10: 5, 11: 6,
};
const ASS_TO_LEGACY: Record<number, number> = {
  1: 1, 2: 2, 3: 3, 4: 9, 5: 10, 6: 11, 7: 5, 8: 6, 9: 7,
};

export const DEFAULT_SUBTITLE_STYLE: SubtitleStyle = Object.freeze({
  fontName: "Arial",
  fontSize: 18,
  fontColor: "#FFFFFF",
  outlineColor: "#000000",
  outlineWidth: 2,
  alignment: 2,
  marginV: 20,
  marginL: 20,
  marginR: 20,
});

const FIELDS = [
  "Name", "Fontname", "Fontsize", "PrimaryColour", "SecondaryColour", "OutlineColour", "BackColour",
  "Bold", "Italic", "Underline", "StrikeOut", "ScaleX", "ScaleY", "Spacing", "Angle", "BorderStyle",
  "Outline", "Shadow", "Alignment", "MarginL", "MarginR", "MarginV", "Encoding",
] as const;
export type AssStyleField = (typeof FIELDS)[number];
export const ASS_STYLE_FORMAT = FIELDS.join(", ");
/** Index of each field in a standard V4+ `Style:` row. */
export const ASS_FIELD = Object.freeze(
  Object.fromEntries(FIELDS.map((field, index) => [field, index])),
) as Readonly<Record<AssStyleField, number>>;
const COLOR_FIELDS = new Set([3, 4, 5, 6]);
const FLAG_FIELDS = new Set([7, 8, 9, 10]);
const INTEGER_FIELDS = new Set([7, 8, 9, 10, 15, 18, 19, 20, 21, 22]);

function numberText(value: number): string {
  // Avoid floating point noise after a round trip through legacy coordinates.
  return String(Math.round(value * 1e6) / 1e6);
}

function safeName(value: string, label: string): string {
  if (!value || value.length > 200 || /[\u0000-\u001f\u007f,\\{}\[\]]/.test(value) || value.startsWith("*")) {
    throw new Error(`${label}不能为空，且不能包含逗号、换行、反斜杠或 ASS 控制符`);
  }
  return value;
}

function assColor(value: string): string {
  if (/^#[0-9a-f]{6}$/i.test(value)) {
    return `&H00${value.slice(5, 7)}${value.slice(3, 5)}${value.slice(1, 3)}`.toUpperCase();
  }
  const match = /^&H([0-9a-f]{6}|[0-9a-f]{8})&?$/i.exec(value);
  if (!match) throw new Error("ASS 颜色须使用 &HAABBGGRR 或 &HBBGGRR 十六进制格式");
  return `&H${match[1].padStart(8, "0").toUpperCase()}`;
}

function htmlColor(value: string): string {
  const hex = assColor(value).slice(4);
  return `#${hex.slice(4, 6)}${hex.slice(2, 4)}${hex.slice(0, 2)}`;
}

function canonicalSource(values: string[]): string {
  return `[V4+ Styles]\nFormat: ${ASS_STYLE_FORMAT}\nStyle: ${values.join(",")}`;
}

function validateValues(values: string[]): string[] {
  if (values.length !== FIELDS.length) throw new Error("Style 必须包含完整的 23 个字段；字体和样式名中不能使用逗号");
  return values.map((raw, index) => {
    const value = raw.trim();
    const field = FIELDS[index];
    if (index < 2) return safeName(value, index === 0 ? "样式名" : "字体名");
    if (COLOR_FIELDS.has(index)) return assColor(value);
    if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/.test(value)) throw new Error(`${field} 必须是有效数字`);
    const numeric = Number(value);
    if (!Number.isFinite(numeric) || Math.abs(numeric) > 1_000_000) throw new Error(`${field} 数值超出有效范围`);
    if (INTEGER_FIELDS.has(index) && !Number.isInteger(numeric)) throw new Error(`${field} 必须是整数`);
    if (FLAG_FIELDS.has(index) && ![0, -1, 1].includes(numeric)) throw new Error(`${field} 只能为 0（关闭）、-1 或 1（开启）`);
    if (index === 15 && numeric !== 1 && numeric !== 3) throw new Error("BorderStyle 只能为 1（描边）或 3（背景框）");
    if (index === 18 && (numeric < 1 || numeric > 9)) throw new Error("Alignment 必须为 1 到 9（数字键盘方位）");
    if ([2, 11, 12].includes(index) && numeric <= 0) throw new Error(`${field} 必须大于 0`);
    if ([16, 17, 19, 20, 21, 22].includes(index) && numeric < 0) throw new Error(`${field} 不能为负数`);
    return INTEGER_FIELDS.has(index) ? String(numeric) : value;
  });
}

function readValues(code: string): string[] {
  if (typeof code !== "string" || code.length > 16_384) throw new Error("ASS 样式代码不能为空且长度不能超过 16384 个字符");
  let format: string[] | undefined;
  let values: string[] | undefined;
  let hasSection = false;
  for (const rawLine of code.replace(/^\uFEFF/, "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith(";")) continue;
    if (/^\[V4\+ Styles\]$/i.test(line)) {
      if (hasSection || format || values) throw new Error("只能编辑一个 [V4+ Styles] 样式段");
      hasSection = true;
      continue;
    }
    if (/^Format:/i.test(line)) {
      if (format || values) throw new Error("Format 必须位于 Style 前，且只能出现一次");
      format = line.slice(line.indexOf(":") + 1).split(",").map((field) => field.trim().toLowerCase());
      if (format.length !== FIELDS.length || new Set(format).size !== FIELDS.length || FIELDS.some((field) => !format.includes(field.toLowerCase()))) {
        throw new Error("Format 必须包含标准 V4+ 样式的全部 23 个字段，且不能重复");
      }
      continue;
    }
    if (/^Style:/i.test(line)) {
      if (values) throw new Error("目前仅支持一条 Style 样式，请删除额外样式");
      const row = line.slice(line.indexOf(":") + 1).split(",");
      if (row.length !== FIELDS.length) throw new Error("Style 必须包含完整的 23 个字段；字体和样式名中不能使用逗号");
      values = format ? FIELDS.map((field) => row[format.indexOf(field.toLowerCase())]) : row;
      continue;
    }
    throw new Error("只能填写 [V4+ Styles]、Format 和 Style；不能加入脚本设置或字幕事件");
  }
  if (!values) throw new Error("缺少 Style: 样式行");
  return validateValues(values);
}

function styleFromValues(values: string[]): SubtitleStyle {
  return {
    fontName: values[1],
    fontSize: Number(values[2]) / SCALE_Y,
    fontColor: htmlColor(values[3]),
    outlineColor: htmlColor(values[5]),
    outlineWidth: Number(values[16]) / SCALE_Y,
    alignment: ASS_TO_LEGACY[Number(values[18])],
    marginL: Number(values[19]) / SCALE_X,
    marginR: Number(values[20]) / SCALE_X,
    marginV: Number(values[21]) / SCALE_Y,
    assStyle: canonicalSource(values),
  };
}

/** Parse a single V4+ style; coordinates are always 1920 × 1080. */
export function parseAssStyle(code: string): SubtitleStyle {
  return styleFromValues(readValues(code));
}

function legacyValues(style: SubtitleStyle): string[] {
  const alignment = LEGACY_TO_ASS[style.alignment];
  if (!alignment) throw new Error("字幕对齐方式无效");
  return validateValues([
    "Default", style.fontName, numberText(style.fontSize * SCALE_Y),
    assColor(style.fontColor), "&H00FFFFFF", assColor(style.outlineColor), "&H00000000",
    "0", "0", "0", "0", "100", "100", "0", "0", "1",
    numberText(style.outlineWidth * SCALE_Y), "0", String(alignment),
    numberText(Math.round(style.marginL * SCALE_X)), numberText(Math.round(style.marginR * SCALE_X)),
    numberText(Math.round(style.marginV * SCALE_Y)), "1",
  ]);
}

export function serializeAssStyle(style: SubtitleStyle): string {
  return canonicalSource(style.assStyle ? readValues(style.assStyle) : legacyValues(style));
}

/** Raw, validated V4+ field values in standard Format order. */
export function assStyleFields(style: SubtitleStyle): string[] {
  return readValues(serializeAssStyle(style));
}

/** Replace raw fields by index, keeping every field that is not named. */
export function updateAssFields(style: SubtitleStyle, fields: Readonly<Record<number, string>>): SubtitleStyle {
  const values = assStyleFields(style);
  for (const [index, value] of Object.entries(fields)) values[Number(index)] = value;
  return styleFromValues(validateValues(values));
}

/** Apply a picker color while keeping the field's existing transparency. */
export function setAssColor(previous: string, html: string): string {
  return `&H${assColor(previous).slice(2, 4)}${assColor(html).slice(4)}`;
}

/** CSS equivalent of an ASS color; ASS alpha 00 is opaque and FF transparent. */
export function cssColor(value: string): string {
  const hex = assColor(value).slice(2);
  const [alpha, blue, green, red] = [0, 2, 4, 6].map((offset) => parseInt(hex.slice(offset, offset + 2), 16));
  return `rgba(${red}, ${green}, ${blue}, ${Math.round((1 - alpha / 255) * 1000) / 1000})`;
}

export interface SubtitleStylePreset {
  id: string;
  name: string;
  /** Fontsize through MarginV (fields 2–21). Name, font and encoding are kept. */
  fields: string;
}

// Values are 1920 × 1080 script coordinates. BorderStyle 3 draws a box in
// OutlineColour, padded by Outline; BackColour is the shadow.
export const SUBTITLE_STYLE_PRESETS: readonly SubtitleStylePreset[] = Object.freeze([
  { id: "classic", name: "经典白字", fields: "64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,4,2,2,100,100,60" },
  { id: "bold", name: "醒目粗体", fields: "72,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,6,0,2,100,100,70" },
  { id: "variety", name: "综艺黄字", fields: "76,&H0000E6FF,&H00FFFFFF,&H00000000,&H64000000,-1,0,0,0,100,100,1,0,1,6,3,2,100,100,70" },
  { id: "box", name: "半透明底框", fields: "60,&H00FFFFFF,&H00FFFFFF,&H64000000,&H64000000,0,0,0,0,100,100,0,0,3,12,0,2,100,100,60" },
  { id: "cinema", name: "电影细字", fields: "56,&H00F0F0F0,&H00FFFFFF,&H00000000,&H96000000,0,0,0,0,100,100,1,0,1,2,2,2,100,100,50" },
  { id: "pink", name: "粉色描边", fields: "68,&H00FFFFFF,&H00FFFFFF,&H00B469FF,&H00000000,-1,0,0,0,100,100,0,0,1,6,0,2,100,100,70" },
  { id: "neon", name: "霓虹蓝光", fields: "68,&H00FFFFFF,&H00FFFFFF,&H00FFC800,&H64FFC800,-1,0,0,0,100,100,1,0,1,5,3,2,100,100,70" },
  { id: "top", name: "顶部标题", fields: "56,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,4,2,8,100,100,50" },
].map((preset) => Object.freeze(preset)));

export function applySubtitlePreset(style: SubtitleStyle, preset: SubtitleStylePreset): SubtitleStyle {
  const values = assStyleFields(style);
  values.splice(ASS_FIELD.Fontsize, ASS_FIELD.Encoding - ASS_FIELD.Fontsize, ...preset.fields.split(","));
  return styleFromValues(validateValues(values));
}

/** The preset whose fields the style currently matches exactly, if any. */
export function matchSubtitlePreset(style: SubtitleStyle): SubtitleStylePreset | undefined {
  const fields = assStyleFields(style).slice(ASS_FIELD.Fontsize, ASS_FIELD.Encoding).join(",");
  return SUBTITLE_STYLE_PRESETS.find((preset) => preset.fields === fields);
}

export function loadSubtitleStyle(roomId: string): SubtitleStyle {
  try {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem(`subtitle_style_${roomId}`) : null;
    if (saved) {
      const style = JSON.parse(saved);
      if (!style || typeof style !== "object" || Array.isArray(style)) throw new Error("字幕样式数据无效");
      return parseAssStyle(serializeAssStyle({ ...DEFAULT_SUBTITLE_STYLE, ...style }));
    }
  } catch {
    // Corrupt, obsolete, or unavailable storage must not prevent opening a clip.
  }
  return parseAssStyle(serializeAssStyle(DEFAULT_SUBTITLE_STYLE));
}

export function saveSubtitleStyle(roomId: string, style: SubtitleStyle): void {
  const normalized = parseAssStyle(serializeAssStyle(style));
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(`subtitle_style_${roomId}`, JSON.stringify(normalized));
  }
}

function assTime(centiseconds: number): string {
  const hours = Math.floor(centiseconds / 360_000);
  const minutes = Math.floor(centiseconds / 6_000) % 60;
  const seconds = Math.floor(centiseconds / 100) % 60;
  return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(centiseconds % 100).padStart(2, "0")}`;
}

function literalText(text: string): string {
  // ASS uses escaped braces. A word joiner prevents literal \N, \h,
  // or \n from becoming ASS escapes, following FFmpeg's plain-text escaping.
  return text.replace(/\\/g, "\\\u2060").replace(/{/g, "\\{").replace(/}/g, "\\}")
    .replace(/\r\n?|\n/g, "\\N").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");
}

function cueText(text: string): string {
  // SRT supports these simple formatting tags. Everything else remains text;
  // the same generated ASS is passed verbatim to the preview and export.
  let result = "";
  let offset = 0;
  let color: string | undefined;
  const colors: (string | undefined)[] = [];
  const namedColors: Record<string, string> = {
    black: "#000000", silver: "#C0C0C0", gray: "#808080", white: "#FFFFFF",
    maroon: "#800000", red: "#FF0000", purple: "#800080", fuchsia: "#FF00FF",
    green: "#008000", lime: "#00FF00", olive: "#808000", yellow: "#FFFF00",
    navy: "#000080", blue: "#0000FF", teal: "#008080", aqua: "#00FFFF",
    orange: "#FFA500",
  };
  const tags = /<\s*(\/?)\s*(b|i|u|s|br|font)\b([^<>]*)>/gi;
  for (const tag of text.matchAll(tags)) {
    result += literalText(text.slice(offset, tag.index));
    const name = tag[2].toLowerCase();
    if (name === "font") {
      let nextColor = color;
      if (tag[1]) {
        if (colors.length > 0) nextColor = colors.pop();
      } else {
        colors.push(color);
        const attribute = /\bcolor\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>/]+))/i.exec(tag[3]);
        if (attribute) {
          let html = (attribute[1] ?? attribute[2] ?? attribute[3]).trim().toLowerCase();
          html = namedColors[html] ?? html;
          if (/^#[0-9a-f]{3}$/i.test(html)) html = "#" + [...html.slice(1)].map((digit) => digit + digit).join("");
          if (/^#[0-9a-f]{6}$/i.test(html)) nextColor = assColor(html).slice(4);
        }
      }
      if (nextColor !== color) result += nextColor ? `{\\c&H${nextColor}&}` : "{\\c}";
      color = nextColor;
    } else {
      result += name === "br" ? "\\N" : `{\\${name}${tag[1] ? "0" : "1"}}`;
    }
    offset = tag.index + tag[0].length;
  }
  return result + literalText(text.slice(offset));
}

/** Build the generated subset shared by native preview/burn-in and portable ASS export. */
export function buildSubtitleAss(cues: readonly SubtitleCue[], style: SubtitleStyle): string {
  const values = readValues(serializeAssStyle(style));
  const ordered = cues.map((cue, index) => ({ cue, index }))
    .filter(({ cue }) => Number.isFinite(cue.startTime) && Number.isFinite(cue.endTime) && cue.startTime >= 0 && cue.endTime > cue.startTime)
    .sort((a, b) => a.cue.startTime - b.cue.startTime || (a.cue.layerOrder ?? a.index) - (b.cue.layerOrder ?? b.index) || a.index - b.index);
  const events: string[] = [];
  for (const { cue } of ordered) {
    const start = Math.round(cue.startTime * 100);
    const end = Math.round(cue.endTime * 100);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end <= start) continue;
    // All cues share a layer for native subtitle collision avoidance. LayerOrder
    // only breaks ties in read order; distinct layers would overlap instead.
    events.push(`Dialogue: 0,${assTime(start)},${assTime(end)},${values[0]},,0,0,0,,${cueText(cue.text)}`);
  }
  return `[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n${canonicalSource(values)}\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n${events.join("\n")}\n`;
}
