// The cover editor's text and sticker elements and their geometry, in cover
// pixels. Free of DOM access so tests can load it; callers measure text.

export const COVER_WIDTH = 1920;
export const COVER_HEIGHT = 1080;
/** The homepage feed shows the middle 4:3 of the cover. */
export const HOMEPAGE_CROP = {
  x: (COVER_WIDTH - (COVER_HEIGHT * 4) / 3) / 2,
  y: 0,
  width: (COVER_HEIGHT * 4) / 3,
  height: COVER_HEIGHT,
};
export const LINE_HEIGHT = 1.25;

export interface TextStyle {
  fill: string;
  /** Outline drawn outside the glyphs, or none. */
  stroke: string | null;
  /** Outline width as a fraction of the font size. */
  strokeWidth: number;
  /** A rounded box behind the text, or none. */
  background: string | null;
  shadow: boolean;
}

interface Placement {
  id: number;
  /** Center. */
  x: number;
  y: number;
  /** Degrees, clockwise. */
  rotation: number;
}

export interface TextElement extends Placement, TextStyle {
  kind: "text";
  content: string;
  /** Font family name; empty for the default sans-serif. */
  font: string;
  fontSize: number;
  bold: boolean;
  align: "left" | "center" | "right";
}

export interface StickerElement extends Placement {
  kind: "sticker";
  /** Image URL. */
  src: string;
  /** The longer side. */
  size: number;
  /** Width over height. */
  aspect: number;
  flip: boolean;
}

export type CoverElement = TextElement | StickerElement;

/** Width of `text` drawn in the CSS `font`. */
export type Measure = (text: string, font: string) => number;

export const DEFAULT_FONT_STACK =
  '"PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans CJK SC", "Source Han Sans SC", sans-serif';

export const TEXT_STYLES: (TextStyle & { id: string; label: string })[] = [
  { id: "yellow", label: "醒目黄", fill: "#FFE14D", stroke: "#1A1A1A", strokeWidth: 0.1, background: null, shadow: false },
  { id: "white", label: "白字黑边", fill: "#FFFFFF", stroke: "#1A1A1A", strokeWidth: 0.1, background: null, shadow: false },
  { id: "red", label: "红字白边", fill: "#FF3B30", stroke: "#FFFFFF", strokeWidth: 0.1, background: null, shadow: true },
  { id: "orange", label: "橙字白边", fill: "#FF7F00", stroke: "#FFFFFF", strokeWidth: 0.1, background: null, shadow: true },
  { id: "pink", label: "粉字白边", fill: "#FF4F9A", stroke: "#FFFFFF", strokeWidth: 0.1, background: null, shadow: true },
  { id: "blue", label: "蓝字白边", fill: "#0A84FF", stroke: "#FFFFFF", strokeWidth: 0.1, background: null, shadow: true },
  { id: "tag-yellow", label: "黄底黑字", fill: "#1A1A1A", stroke: null, strokeWidth: 0.1, background: "#FFE14D", shadow: false },
  { id: "tag-red", label: "红底白字", fill: "#FFFFFF", stroke: null, strokeWidth: 0.1, background: "#FF3B30", shadow: false },
  { id: "tag-black", label: "黑底黄字", fill: "#FFE14D", stroke: null, strokeWidth: 0.1, background: "#1A1A1A", shadow: false },
  { id: "shadow", label: "白字投影", fill: "#FFFFFF", stroke: null, strokeWidth: 0.1, background: null, shadow: true },
];

export function styleOf(style: TextStyle): TextStyle {
  const { fill, stroke, strokeWidth, background, shadow } = style;
  return { fill, stroke, strokeWidth, background, shadow };
}

export function sameStyle(a: TextStyle, b: TextStyle): boolean {
  return (
    a.fill.toUpperCase() === b.fill.toUpperCase() &&
    (a.stroke ?? "").toUpperCase() === (b.stroke ?? "").toUpperCase() &&
    (!a.stroke || a.strokeWidth === b.strokeWidth) &&
    (a.background ?? "").toUpperCase() === (b.background ?? "").toUpperCase() &&
    a.shadow === b.shadow
  );
}

/** CSS font-family for a family name, falling back to the default. */
export function fontFamily(font: string): string {
  return font ? `"${font.replace(/["\\]/g, "\\$&")}", ${DEFAULT_FONT_STACK}` : DEFAULT_FONT_STACK;
}

export function textFont(text: Pick<TextElement, "font" | "fontSize" | "bold">): string {
  return `${text.bold ? "bold " : ""}${text.fontSize}px ${fontFamily(text.font)}`;
}

export interface TextLayout {
  font: string;
  lines: string[];
  lineHeight: number;
  /** The widest line. */
  textWidth: number;
  textHeight: number;
  padX: number;
  padY: number;
  /** Including the outline or background box. */
  width: number;
  height: number;
}

export function layoutText(text: TextElement, measure: Measure): TextLayout {
  const font = textFont(text);
  const lines = text.content.split("\n");
  const lineHeight = text.fontSize * LINE_HEIGHT;
  const textWidth = Math.max(text.fontSize / 2, ...lines.map((line) => measure(line, font)));
  const textHeight = lines.length * lineHeight;
  const outline = text.stroke ? text.strokeWidth * text.fontSize : 0;
  const padX = text.background ? text.fontSize * 0.35 : outline + text.fontSize * 0.05;
  const padY = text.background ? text.fontSize * 0.12 : outline;
  return {
    font,
    lines,
    lineHeight,
    textWidth,
    textHeight,
    padX,
    padY,
    width: textWidth + padX * 2,
    height: textHeight + padY * 2,
  };
}

export function stickerSize(sticker: Pick<StickerElement, "size" | "aspect">): { width: number; height: number } {
  return sticker.aspect >= 1
    ? { width: sticker.size, height: sticker.size / sticker.aspect }
    : { width: sticker.size * sticker.aspect, height: sticker.size };
}

export function elementSize(element: CoverElement, measure: Measure): { width: number; height: number } {
  if (element.kind === "sticker") return stickerSize(element);
  const { width, height } = layoutText(element, measure);
  return { width, height };
}

/** The point relative to the element's center, in its unrotated frame. */
export function toLocal(element: Placement, x: number, y: number): { x: number; y: number } {
  const angle = (-element.rotation * Math.PI) / 180;
  const dx = x - element.x;
  const dy = y - element.y;
  return {
    x: dx * Math.cos(angle) - dy * Math.sin(angle),
    y: dx * Math.sin(angle) + dy * Math.cos(angle),
  };
}

/** The topmost element under the point; `slop` widens every element. */
export function elementAt(
  elements: CoverElement[],
  x: number,
  y: number,
  measure: Measure,
  slop = 0,
): CoverElement | null {
  for (let index = elements.length - 1; index >= 0; index--) {
    const element = elements[index];
    const { width, height } = elementSize(element, measure);
    const local = toLocal(element, x, y);
    if (Math.abs(local.x) <= width / 2 + slop && Math.abs(local.y) <= height / 2 + slop) return element;
  }
  return null;
}

/** Pulls a center within `threshold` of the cover's middle lines onto them. */
export function snapToCenter(x: number, y: number, threshold: number) {
  const snapX = Math.abs(x - COVER_WIDTH / 2) <= threshold;
  const snapY = Math.abs(y - COVER_HEIGHT / 2) <= threshold;
  return { x: snapX ? COVER_WIDTH / 2 : x, y: snapY ? COVER_HEIGHT / 2 : y, snapX, snapY };
}

/** Normalized to [-180, 180]: to multiples of `step`, or else near right angles. */
export function snapRotation(degrees: number, step?: number): number {
  let angle = ((((degrees + 180) % 360) + 360) % 360) - 180;
  const target = step ? step : 90;
  const nearest = Math.round(angle / target) * target;
  if (step || Math.abs(angle - nearest) <= 4) angle = nearest;
  return angle + 0;
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** The size fields of `origin` scaled by `ratio`, within usable limits. */
export function scaledSize(origin: CoverElement, ratio: number): Partial<TextElement> | Partial<StickerElement> {
  return origin.kind === "text"
    ? { fontSize: clamp(Math.round(origin.fontSize * ratio), 12, 800) }
    : { size: clamp(Math.round(origin.size * ratio), 24, 1600) };
}

/** The cover's middle, moved down-right past elements already placed there. */
export function freePosition(elements: CoverElement[]): { x: number; y: number } {
  let x = COVER_WIDTH / 2;
  let y = COVER_HEIGHT / 2;
  for (let step = 0; step < 8 && elements.some((e) => Math.abs(e.x - x) < 1 && Math.abs(e.y - y) < 1); step++) {
    x += 60;
    y += 60;
  }
  return { x, y };
}

export function nextId(elements: CoverElement[]): number {
  return Math.max(0, ...elements.map((element) => element.id)) + 1;
}

export function newText(elements: CoverElement[], style: TextStyle, font = ""): TextElement {
  return {
    kind: "text",
    id: nextId(elements),
    ...freePosition(elements),
    rotation: 0,
    content: "请输入文字",
    font,
    fontSize: 120,
    bold: true,
    align: "center",
    ...styleOf(style),
  };
}

export function newSticker(elements: CoverElement[], src: string, aspect = 1, size = 280): StickerElement {
  return { kind: "sticker", id: nextId(elements), ...freePosition(elements), rotation: 0, src, size, aspect, flip: false };
}

/** Where `width`×`height` lands when it covers or fits inside the cover, centered. */
export function fitRect(width: number, height: number, mode: "cover" | "contain") {
  const scale = (mode === "cover" ? Math.max : Math.min)(COVER_WIDTH / width, COVER_HEIGHT / height);
  return {
    x: (COVER_WIDTH - width * scale) / 2,
    y: (COVER_HEIGHT - height * scale) / 2,
    width: width * scale,
    height: height * scale,
  };
}

/** Moves the element at `index` to `target`, clamped to the stack. */
export function reorder<T>(items: T[], index: number, target: number): T[] {
  const next = [...items];
  const [item] = next.splice(index, 1);
  next.splice(clamp(target, 0, next.length), 0, item);
  return next;
}
