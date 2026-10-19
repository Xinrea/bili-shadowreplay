// The cover editor's model, free of DOM access so tests can load it; callers
// measure text.
//
// Bilibili shows two covers: 4:3 in the homepage feed and 16:9 elsewhere.
// Each frames the background picture, the scene, with its own zoom and
// position; scene units are the picture scaled to SCENE_HEIGHT tall. Text
// and stickers sit on the covers instead, in the 16:9 cover's pixels with
// the 4:3 cover showing the middle of them, so they keep their place and
// size in both whatever each picture's framing. Each cover has its own
// elements; edits made while the covers are synced carry over to the twin,
// the element with the same id on the other cover.

export const SCENE_HEIGHT = 1080;
export const MAX_ZOOM = 4;
export const LINE_HEIGHT = 1.25;

export type Ratio = "home" | "wide";

/** Pixel size of each saved cover. */
export const COVER_SIZES: Record<Ratio, { width: number; height: number }> = {
  home: { width: 1440, height: 1080 },
  wide: { width: 1920, height: 1080 },
};

export const aspectOf = (ratio: Ratio) => COVER_SIZES[ratio].width / COVER_SIZES[ratio].height;

export interface Size {
  width: number;
  height: number;
}

export interface Rect extends Size {
  x: number;
  y: number;
}

/** A cover's view of the scene: its center and its zoom over covering it. */
export interface Frame {
  x: number;
  y: number;
  zoom: number;
}

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

/** Pulls a point within `threshold` of the lines through `center` onto them. */
export function snapToCenter(x: number, y: number, center: { x: number; y: number }, threshold: number) {
  const snapX = Math.abs(x - center.x) <= threshold;
  const snapY = Math.abs(y - center.y) <= threshold;
  return { x: snapX ? center.x : x, y: snapY ? center.y : y, snapX, snapY };
}

/** `degrees` within [-180, 180). */
export function normalizeAngle(degrees: number): number {
  return ((((degrees + 180) % 360) + 360) % 360) - 180;
}

/** Normalized to [-180, 180]: to multiples of `step`, or else near right angles. */
export function snapRotation(degrees: number, step?: number): number {
  let angle = normalizeAngle(degrees);
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

export type ElementChanges = Partial<TextElement> | Partial<StickerElement>;

/**
 * `changes` to `element` as they apply to its twin on the other cover, which
 * may have been placed apart: moved, turned and resized by as much, and
 * otherwise matched where they change something.
 */
export function twinChanges(element: CoverElement, changes: ElementChanges, twin: CoverElement): ElementChanges {
  const before = element as unknown as Record<string, unknown>;
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(changes)) {
    if (key === "x" || key === "y") {
      const extent = key === "x" ? ELEMENT_SPACE.width : ELEMENT_SPACE.height;
      result[key] = clamp(twin[key] + (value as number) - element[key], 0, extent);
    } else if (key === "rotation") {
      result.rotation = normalizeAngle(twin.rotation + (value as number) - element.rotation);
    } else if (key === "fontSize" || key === "size") {
      Object.assign(result, scaledSize(twin, (value as number) / (before[key] as number)));
    } else if (value !== before[key]) {
      result[key] = value;
    }
  }
  return result as ElementChanges;
}

/** `at`, moved down-right past elements already placed there. */
export function freePosition(elements: CoverElement[], at: { x: number; y: number }): { x: number; y: number } {
  let { x, y } = at;
  for (let step = 0; step < 8 && elements.some((e) => Math.abs(e.x - x) < 1 && Math.abs(e.y - y) < 1); step++) {
    x += 60;
    y += 60;
  }
  return { x, y };
}

export function nextId(elements: CoverElement[]): number {
  return Math.max(0, ...elements.map((element) => element.id)) + 1;
}

export function newText(
  elements: CoverElement[],
  at: { x: number; y: number },
  style: TextStyle,
  font = "",
): TextElement {
  return {
    kind: "text",
    id: nextId(elements),
    ...freePosition(elements, at),
    rotation: 0,
    content: "请输入文字",
    font,
    fontSize: 120,
    bold: true,
    align: "center",
    ...styleOf(style),
  };
}

export function newSticker(
  elements: CoverElement[],
  at: { x: number; y: number },
  src: string,
  aspect = 1,
  size = 280,
): StickerElement {
  return { kind: "sticker", id: nextId(elements), ...freePosition(elements, at), rotation: 0, src, size, aspect, flip: false };
}

/** Where `width`×`height` lands when it covers or fits inside `target`, centered. */
export function fitRect(width: number, height: number, target: Size, mode: "cover" | "contain"): Rect {
  const scale = (mode === "cover" ? Math.max : Math.min)(target.width / width, target.height / height);
  return {
    x: (target.width - width * scale) / 2,
    y: (target.height - height * scale) / 2,
    width: width * scale,
    height: height * scale,
  };
}

/** The scene of a `width`×`height` picture; 16:9 without one. */
export function sceneOf(width = 16, height = 9): Size {
  return { width: (SCENE_HEIGHT * width) / height, height: SCENE_HEIGHT };
}

/** Width of a frame of `aspect` that just covers the scene. */
function coverWidth(scene: Size, aspect: number): number {
  return Math.min(scene.width, scene.height * aspect);
}

/** Zooming out until the whole picture shows, on a blurred fill. */
export function minZoom(scene: Size, aspect: number): number {
  return coverWidth(scene, aspect) / Math.max(scene.width, scene.height * aspect);
}

/** The part of the scene a frame shows. */
export function frameRect(scene: Size, aspect: number, frame: Frame): Rect {
  const width = coverWidth(scene, aspect) / frame.zoom;
  const height = width / aspect;
  return { x: frame.x - width / 2, y: frame.y - height / 2, width, height };
}

/** `frame` with its zoom in range and the picture kept under it where it fits. */
export function clampFrame(scene: Size, aspect: number, frame: Frame): Frame {
  const zoom = clamp(frame.zoom, minZoom(scene, aspect), MAX_ZOOM);
  const width = coverWidth(scene, aspect) / zoom;
  const along = (center: number, size: number, extent: number) =>
    size <= extent ? clamp(center, size / 2, extent - size / 2) : extent / 2;
  return { zoom, x: along(frame.x, width, scene.width), y: along(frame.y, width / aspect, scene.height) };
}

/** Centered, covering the scene unless that crops away more than half of it. */
export function defaultFrame(scene: Size, aspect: number): Frame {
  const width = coverWidth(scene, aspect);
  const kept = (width * (width / aspect)) / (scene.width * scene.height);
  return { x: scene.width / 2, y: scene.height / 2, zoom: kept < 0.5 ? minZoom(scene, aspect) : 1 };
}

/** Zoomed to `zoom` with the scene point `anchor` staying put in the frame. */
export function zoomFrame(
  scene: Size,
  aspect: number,
  frame: Frame,
  zoom: number,
  anchor: { x: number; y: number } = frame,
): Frame {
  const next = clamp(zoom, minZoom(scene, aspect), MAX_ZOOM);
  const ratio = frame.zoom / next;
  return clampFrame(scene, aspect, {
    zoom: next,
    x: anchor.x + (frame.x - anchor.x) * ratio,
    y: anchor.y + (frame.y - anchor.y) * ratio,
  });
}

/** Elements are placed in the pixels of the 16:9 cover. */
export const ELEMENT_SPACE: Size = COVER_SIZES.wide;

/** The part of element space a cover shows: all of it, or its middle 4:3. */
export function elementArea(ratio: Ratio): Rect {
  const { width, height } = COVER_SIZES[ratio];
  return { x: (ELEMENT_SPACE.width - width) / 2, y: (ELEMENT_SPACE.height - height) / 2, width, height };
}

/** Moves the element at `index` to `target`, clamped to the stack. */
export function reorder<T>(items: T[], index: number, target: number): T[] {
  const next = [...items];
  const [item] = next.splice(index, 1);
  next.splice(clamp(target, 0, next.length), 0, item);
  return next;
}
