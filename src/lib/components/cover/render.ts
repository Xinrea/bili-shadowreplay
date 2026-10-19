// Draws cover elements onto a canvas; the editor and the saved cover share it.
import {
  COVER_HEIGHT,
  COVER_WIDTH,
  fitRect,
  layoutText,
  stickerSize,
  type CoverElement,
  type Measure,
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

const stickerRequests = new Map<string, Promise<HTMLImageElement>>();
const stickerImages = new Map<string, HTMLImageElement>();

/** The sticker image at `src`, loaded once. */
export function loadSticker(src: string): Promise<HTMLImageElement> {
  let request = stickerRequests.get(src);
  if (!request) {
    request = new Promise((resolve, reject) => {
      const image = new Image();
      // Imported stickers come from the static server; keep the cover exportable.
      image.crossOrigin = "anonymous";
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

// Shadows ignore the transform, so they fall the same way at any rotation.
function setShadow(ctx: CanvasRenderingContext2D, fontSize: number) {
  ctx.shadowColor = "rgba(0, 0, 0, 0.55)";
  ctx.shadowBlur = fontSize * 0.15;
  ctx.shadowOffsetX = fontSize * 0.03;
  ctx.shadowOffsetY = fontSize * 0.05;
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
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(source, 0, 0, width, height);
  return canvas;
}

/**
 * Draws `source` over the whole cover. Another aspect ratio is fitted inside
 * a blurred, darkened copy of itself instead of being stretched.
 */
export function drawBackground(ctx: CanvasRenderingContext2D, source: CanvasImageSource, width: number, height: number) {
  if (Math.abs(width / height - COVER_WIDTH / COVER_HEIGHT) < 0.02) {
    ctx.drawImage(source, 0, 0, COVER_WIDTH, COVER_HEIGHT);
    return;
  }
  // Blur by halving down to a few pixels and scaling back up in steps;
  // canvas filters are missing in WebKit.
  const cover = fitRect(width, height, "cover");
  const k = 480 / COVER_WIDTH;
  let blur = document.createElement("canvas");
  blur.width = 480;
  blur.height = 270;
  const first = blur.getContext("2d");
  first.imageSmoothingQuality = "high";
  first.drawImage(source, cover.x * k, cover.y * k, cover.width * k, cover.height * k);
  for (const [w, h] of [[240, 135], [120, 68], [60, 34], [30, 17], [120, 68], [480, 270]]) {
    blur = resample(blur, w, h);
  }
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(blur, 0, 0, COVER_WIDTH, COVER_HEIGHT);
  ctx.fillStyle = "rgba(0, 0, 0, 0.3)";
  ctx.fillRect(0, 0, COVER_WIDTH, COVER_HEIGHT);
  const contain = fitRect(width, height, "contain");
  ctx.drawImage(source, contain.x, contain.y, contain.width, contain.height);
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
