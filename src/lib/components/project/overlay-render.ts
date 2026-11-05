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
 * symmetric bounds around its saved center. The output canvas clips the
 * composition after scaling. Throws `StickerLoadError` for a sticker that does not
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
    // Keep the PNG centered on the element even when it crosses the frame.
    // Its center is mapped to output coordinates while its dimensions scale
    // uniformly; clipping before this step would move an offscreen element.
    const radiusX = Math.ceil((bounds.right - bounds.left) / 2);
    const radiusY = Math.ceil((bounds.bottom - bounds.top) / 2);
    if (radiusX * 2 > 4096 || radiusY * 2 > 4096) {
      throw new Error("文字或贴纸画面超过 4096 × 4096，请缩小元素后重试");
    }
    const left = element.x - radiusX;
    const top = element.y - radiusY;
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, radiusX * 2);
    canvas.height = Math.max(1, radiusY * 2);
    const ctx = canvas.getContext("2d");
    ctx.translate(-left, -top);
    if (element.kind === "text") drawText(ctx, element);
    else drawSticker(ctx, element, picture);
    images.push({ id: item.id, image: canvas.toDataURL("image/png"), x: left, y: top });
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
  const sx = width / ELEMENT_SPACE.width;
  const sy = height / ELEMENT_SPACE.height;
  const scale = Math.min(sx, sy);
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
  return elements.flatMap((element) => {
    ctx.setTransform(scale, 0, 0, scale, element.x * (sx - scale), element.y * (sy - scale));
    return drawElements(ctx, [element]);
  });
}
