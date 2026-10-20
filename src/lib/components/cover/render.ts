// Draws cover elements onto a canvas; the editor and the saved cover share it.
import {
  fitRect,
  layoutText,
  stickerSize,
  type CoverElement,
  type Measure,
  type Rect,
  type Size,
  type StickerElement,
  type TextElement,
} from "../../cover-elements";
import stickerManifest from "./stickers.json";

export interface StickerGroup {
  name: string;
  stickers: { id: string; glyph: string; name: string }[];
}

export const STICKER_GROUPS: StickerGroup[] = stickerManifest.groups;

let measureContext: CanvasRenderingContext2D | null = null;

export const measure: Measure = (text, font) => {
  measureContext ??= document.createElement("canvas").getContext("2d");
  measureContext.font = font;
  return measureContext.measureText(text).width;
};

export function stickerUrl(id: string): string {
  return `${import.meta.env.BASE_URL}stickers/${id}.svg`;
}

/**
 * Whether `src` needs CORS to keep a canvas exportable. Same-origin files,
 * like the built-in stickers, are loaded without it: some webviews fail
 * CORS requests to their own custom protocol.
 */
export function isCrossOrigin(src: string): boolean {
  return new URL(src, location.href).origin !== location.origin;
}

const stickerRequests = new Map<string, Promise<HTMLImageElement>>();
const stickerImages = new Map<string, HTMLImageElement>();

/** The sticker image at `src`, loaded once. */
export function loadSticker(src: string): Promise<HTMLImageElement> {
  let request = stickerRequests.get(src);
  if (!request) {
    request = new Promise((resolve, reject) => {
      const image = new Image();
      if (isCrossOrigin(src)) image.crossOrigin = "anonymous";
      image.onload = () => {
        stickerImages.set(src, image);
        resolve(image);
      };
      image.onerror = () => {
        stickerRequests.delete(src);
        reject(new Error(`Failed to load sticker ${src}`));
      };
      image.src = src;
    });
    stickerRequests.set(src, request);
  }
  return request;
}

/** Runs `draw` once per animation frame however often it is scheduled. */
export function frameScheduler(draw: () => void): { schedule: () => void; cancel: () => void } {
  let request: number | null = null;
  return {
    schedule() {
      if (request !== null) return;
      request = requestAnimationFrame(() => {
        request = null;
        draw();
      });
    },
    cancel() {
      if (request !== null) cancelAnimationFrame(request);
      request = null;
    },
  };
}

/** Schedules a draw once `pending` stickers arrive; only for those that did, or a failed one would loop. */
export function redrawWhenLoaded(pending: Promise<unknown>[], schedule: () => void) {
  if (!pending.length) return;
  void Promise.all(pending.map((request) => request.then(() => true, () => false))).then(
    (loaded) => loaded.some(Boolean) && schedule(),
  );
}

function roundedRect(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius: number) {
  const r = Math.min(radius, width / 2, height / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + width, y, x + width, y + height, r);
  ctx.arcTo(x + width, y + height, x, y + height, r);
  ctx.arcTo(x, y + height, x, y, r);
  ctx.arcTo(x, y, x + width, y, r);
  ctx.closePath();
}

// Shadows ignore the transform, so they fall the same way at any rotation,
// but they must follow its scale.
function setShadow(ctx: CanvasRenderingContext2D, fontSize: number) {
  const { a, b } = ctx.getTransform();
  const size = fontSize * Math.hypot(a, b);
  ctx.shadowColor = "rgba(0, 0, 0, 0.55)";
  ctx.shadowBlur = size * 0.15;
  ctx.shadowOffsetX = size * 0.03;
  ctx.shadowOffsetY = size * 0.05;
}

function clearShadow(ctx: CanvasRenderingContext2D) {
  ctx.shadowColor = "transparent";
}

export function drawText(ctx: CanvasRenderingContext2D, text: TextElement) {
  const layout = layoutText(text, measure);
  ctx.save();
  ctx.translate(text.x, text.y);
  ctx.rotate((text.rotation * Math.PI) / 180);
  if (text.background) {
    roundedRect(ctx, -layout.width / 2, -layout.height / 2, layout.width, layout.height, text.fontSize * 0.18);
    ctx.fillStyle = text.background;
    if (text.shadow) setShadow(ctx, text.fontSize);
    ctx.fill();
    clearShadow(ctx);
  }
  ctx.font = layout.font;
  ctx.textAlign = text.align;
  ctx.textBaseline = "middle";
  const x = text.align === "left" ? -layout.textWidth / 2 : text.align === "right" ? layout.textWidth / 2 : 0;
  const lineY = (index: number) => -layout.textHeight / 2 + layout.lineHeight * (index + 0.5);
  const shadowed = text.shadow && !text.background;
  if (text.stroke) {
    // Twice the width: the fill covers the inner half.
    ctx.lineWidth = text.strokeWidth * text.fontSize * 2;
    ctx.lineJoin = "round";
    ctx.strokeStyle = text.stroke;
    if (shadowed) setShadow(ctx, text.fontSize);
    layout.lines.forEach((line, index) => ctx.strokeText(line, x, lineY(index)));
    clearShadow(ctx);
  } else if (shadowed) {
    setShadow(ctx, text.fontSize);
  }
  ctx.fillStyle = text.fill;
  layout.lines.forEach((line, index) => ctx.fillText(line, x, lineY(index)));
  ctx.restore();
}

export function drawSticker(ctx: CanvasRenderingContext2D, sticker: StickerElement, image: CanvasImageSource) {
  ctx.save();
  ctx.translate(sticker.x, sticker.y);
  ctx.rotate((sticker.rotation * Math.PI) / 180);
  if (sticker.flip) ctx.scale(-1, 1);
  const { width, height } = stickerSize(sticker);
  ctx.drawImage(image, -width / 2, -height / 2, width, height);
  ctx.restore();
}

/**
 * Draws every element but `skip`. Returns stickers that are still loading;
 * draw again once they have.
 */
export function drawElements(ctx: CanvasRenderingContext2D, elements: CoverElement[], skip: number | null = null) {
  const pending: Promise<HTMLImageElement>[] = [];
  for (const element of elements) {
    if (element.id === skip) continue;
    if (element.kind === "text") {
      drawText(ctx, element);
    } else {
      const image = stickerImages.get(element.src);
      if (image) drawSticker(ctx, element, image);
      else pending.push(loadSticker(element.src));
    }
  }
  return pending;
}

function resample(source: CanvasImageSource, width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  const ctx = canvas.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(source, 0, 0, canvas.width, canvas.height);
  return canvas;
}

/** The background picture, copied so later video seeks leave it alone. */
export interface Background {
  image: HTMLCanvasElement;
  /** Fills what a zoomed-out cover shows around the picture. */
  blur: HTMLCanvasElement;
}

const MAX_BACKGROUND_SIDE = 3840;

export function prepareBackground(source: CanvasImageSource, width: number, height: number): Background {
  const fit = Math.min(1, MAX_BACKGROUND_SIDE / Math.max(width, height));
  const image = resample(source, width * fit, height * fit);
  // Blur by halving down to a few pixels and scaling back up in steps;
  // canvas filters are missing in WebKit.
  const small = Math.min(1, 480 / Math.max(width, height));
  let blur = resample(image, width * small, height * small);
  for (const factor of [0.5, 0.5, 0.5, 0.5, 4, 4]) {
    blur = resample(blur, blur.width * factor, blur.height * factor);
  }
  return { image, blur };
}

/**
 * Draws onto the whole canvas a cover showing `view` of the picture's
 * `scene` and `area` of element space. Returns stickers that are still
 * loading; draw again once they have.
 */
export function renderCover(
  ctx: CanvasRenderingContext2D,
  scene: Size,
  view: Rect,
  background: Background | null,
  area: Rect,
  elements: CoverElement[],
  skip: number | null = null,
) {
  const { width, height } = ctx.canvas;
  const k = width / view.width;
  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.imageSmoothingQuality = "high";
  if (!background) {
    ctx.fillStyle = "#2A2F3A";
    ctx.fillRect(0, 0, width, height);
  } else {
    const outside =
      view.x < -0.5 || view.y < -0.5 || view.x + view.width > scene.width + 0.5 || view.y + view.height > scene.height + 0.5;
    if (outside) {
      const fill = fitRect(background.blur.width, background.blur.height, { width, height }, "cover");
      ctx.drawImage(background.blur, fill.x, fill.y, fill.width, fill.height);
      ctx.fillStyle = "rgba(0, 0, 0, 0.3)";
      ctx.fillRect(0, 0, width, height);
    }
    ctx.drawImage(background.image, -view.x * k, -view.y * k, scene.width * k, scene.height * k);
  }
  const e = width / area.width;
  ctx.setTransform(e, 0, 0, e, -area.x * e, -area.y * e);
  const pending = drawElements(ctx, elements, skip);
  ctx.restore();
  return pending;
}

const fontChecks = new Map<string, boolean>();
const FONT_SAMPLE = "mmmmmmmmmmlli WQ@#0 永和九年";
const GENERIC_FONTS = ["monospace", "serif", "sans-serif"];
let genericWidths: number[] | null = null;

/** Whether this webview can draw `family`: some generic fallback changes width. */
export function fontAvailable(family: string): boolean {
  let available = fontChecks.get(family);
  if (available === undefined) {
    genericWidths ??= GENERIC_FONTS.map((generic) => measure(FONT_SAMPLE, `72px ${generic}`));
    const quoted = `"${family.replace(/["\\]/g, "\\$&")}"`;
    available = GENERIC_FONTS.some(
      (generic, index) => measure(FONT_SAMPLE, `72px ${quoted}, ${generic}`) !== genericWidths[index],
    );
    fontChecks.set(family, available);
  }
  return available;
}

/** Common Chinese typefaces by look, each with names across systems. */
export const FONT_PICKS: { label: string; families: string[] }[] = [
  { label: "宋体", families: ["Songti SC", "STSong", "SimSun", "Noto Serif CJK SC", "Source Han Serif SC"] },
  { label: "楷体", families: ["Kaiti SC", "STKaiti", "KaiTi", "AR PL UKai CN"] },
  { label: "仿宋", families: ["STFangsong", "FangSong"] },
  { label: "圆体", families: ["Yuanti SC", "YouYuan"] },
  { label: "行楷", families: ["Xingkai SC", "STXingkai"] },
  { label: "隶书", families: ["Libian SC", "LiSu", "STLiti"] },
  { label: "魏碑", families: ["Weibei SC"] },
  { label: "琥珀", families: ["STHupo"] },
  { label: "彩云", families: ["STCaiyun"] },
  { label: "舒体", families: ["FZShuTi"] },
  { label: "姚体", families: ["FZYaoti"] },
  { label: "手札体", families: ["Hannotate SC"] },
  { label: "翩翩体", families: ["HanziPen SC"] },
  { label: "娃娃体", families: ["Wawati SC"] },
  { label: "雅痞", families: ["Yuppy SC"] },
  { label: "兰亭黑", families: ["Lantinghei SC"] },
  { label: "等线", families: ["DengXian"] },
  { label: "Impact", families: ["Impact"] },
];
