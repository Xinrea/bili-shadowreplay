// Draws a project's text and sticker overlays, on the editor's preview and as
// the pictures an export burns in, with the cover editor's drawing so all of
// them look the same.
import {
  ELEMENT_SPACE,
  elementSize,
  type CoverElement,
  type StickerElement,
  type TextElement,
} from "../../cover-elements";
import type { OverlayImage, ProjectTimeline, StickerOverlay, TextOverlay } from "../../interface";
import { placeOverlays } from "../../project-timeline";
import { drawElements, drawSticker, drawText, loadSticker, measure } from "../cover/render";

export type DrawnOverlay = TextOverlay | StickerOverlay;

/** A sticker overlay whose picture did not load. */
export class StickerLoadError extends Error {
  constructor(readonly overlayId: string) {
    super("贴纸加载失败");
  }
}

/** What the element covers once turned, with room for a text's shadow. */
function drawnBounds(element: CoverElement) {
  const { width, height } = elementSize(element, measure);
  const margin = element.kind === "text" ? element.fontSize * (element.shadow ? 0.3 : 0.08) : 2;
  const angle = (element.rotation * Math.PI) / 180;
  const cos = Math.abs(Math.cos(angle));
  const sin = Math.abs(Math.sin(angle));
  const halfWidth = (width * cos + height * sin) / 2 + margin;
  const halfHeight = (width * sin + height * cos) / 2 + margin;
  return {
    left: element.x - halfWidth,
    top: element.y - halfHeight,
    right: element.x + halfWidth,
    bottom: element.y + halfHeight,
  };
}

/**
 * The picture of each visible text and sticker overlay of `timeline`, cut to
 * the 1920×1080 frame. Throws `StickerLoadError` for a sticker that does not
 * load, rather than leaving it out.
 */
export async function renderOverlayImages(
  timeline: ProjectTimeline,
  stickerUrl: (ref: string) => Promise<string>,
): Promise<OverlayImage[]> {
  const images: OverlayImage[] = [];
  for (const { item } of placeOverlays(timeline)) {
    if (item.kind === "video") continue;
    let element: TextElement | StickerElement = item.element;
    let picture: HTMLImageElement | null = null;
    if (item.kind === "sticker") {
      try {
        const src = await stickerUrl(item.element.src);
        picture = await loadSticker(src);
        element = { ...item.element, src };
      } catch {
        throw new StickerLoadError(item.id);
      }
    }
    const bounds = drawnBounds(element);
    const left = Math.max(0, Math.floor(bounds.left));
    const top = Math.max(0, Math.floor(bounds.top));
    const right = Math.min(ELEMENT_SPACE.width, Math.ceil(bounds.right));
    const bottom = Math.min(ELEMENT_SPACE.height, Math.ceil(bounds.bottom));
    // Off the frame altogether, it is a transparent pixel.
    const visible = right > left && bottom > top;
    const canvas = document.createElement("canvas");
    canvas.width = visible ? right - left : 1;
    canvas.height = visible ? bottom - top : 1;
    const ctx = canvas.getContext("2d");
    if (visible) {
      ctx.translate(-left, -top);
      if (element.kind === "text") drawText(ctx, element);
      else drawSticker(ctx, element, picture);
    }
    images.push({ id: item.id, image: canvas.toDataURL("image/png"), x: visible ? left : 0, y: visible ? top : 0 });
  }
  return images;
}

/**
 * Draws `items` over the whole canvas, which shows the 1920×1080 frame,
 * leaving out item `skip` and stickers whose URL `stickerSrc` does not know
 * yet. Returns stickers still loading; draw again once they have.
 */
export function drawOverlays(
  ctx: CanvasRenderingContext2D,
  items: DrawnOverlay[],
  stickerSrc: (ref: string) => string | undefined,
  skip: string | null = null,
) {
  const { width, height } = ctx.canvas;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const scale = width / ELEMENT_SPACE.width;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  const elements: CoverElement[] = [];
  for (const item of items) {
    if (item.id === skip) continue;
    if (item.kind === "text") {
      elements.push(item.element);
    } else {
      const src = stickerSrc(item.element.src);
      if (src) elements.push({ ...item.element, src });
    }
  }
  return drawElements(ctx, elements);
}
