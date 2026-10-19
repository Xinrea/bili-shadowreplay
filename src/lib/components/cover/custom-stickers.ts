// The user's imported stickers, which the backend keeps in its output
// directory so every client of the server shares them.
import { get_static_url, invoke } from "../../invoker";

export interface CustomSticker {
  id: string;
  url: string;
}

interface StoredSticker {
  id: string;
  /** Relative to the output directory. */
  file: string;
}

/** Longest side of a saved sticker. */
const MAX_SIDE = 1024;
/** Longest side looked at for transparent margins. */
const MAX_SCAN = 2048;

async function resolve(sticker: StoredSticker): Promise<CustomSticker> {
  return { id: sticker.id, url: await get_static_url("output", sticker.file) };
}

export async function loadCustomStickers(): Promise<CustomSticker[]> {
  return Promise.all((await invoke<StoredSticker[]>("get_cover_stickers")).map(resolve));
}

export async function importSticker(file: Blob): Promise<CustomSticker> {
  const image = await stickerPng(file);
  return resolve(await invoke<StoredSticker>("add_cover_sticker", { image }));
}

export function deleteCustomSticker(id: string): Promise<void> {
  return invoke("delete_cover_sticker", { id });
}

function decode(url: string): Promise<HTMLImageElement> {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("无法读取这张图片"));
    image.src = url;
  });
}

/** The opaque part of the canvas, or null when it is all transparent. */
function opaqueBounds(ctx: CanvasRenderingContext2D, width: number, height: number) {
  const { data } = ctx.getImageData(0, 0, width, height);
  let left = width;
  let top = height;
  let right = -1;
  let bottom = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (data[(y * width + x) * 4 + 3] > 8) {
        if (x < left) left = x;
        if (x > right) right = x;
        if (y < top) top = y;
        bottom = y;
      }
    }
  }
  return right < 0 ? null : { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

/**
 * `file` as a PNG data URL without its transparent margins, so the selection
 * box hugs the picture, and at most MAX_SIDE across.
 */
async function stickerPng(file: Blob): Promise<string> {
  // Revoked only after drawing: some WebKit builds draw a revoked image blank.
  const url = URL.createObjectURL(file);
  try {
    return trimmedPng(await decode(url));
  } finally {
    URL.revokeObjectURL(url);
  }
}

function trimmedPng(image: HTMLImageElement): string {
  // SVGs without a size report zero.
  const naturalWidth = image.naturalWidth || 512;
  const naturalHeight = image.naturalHeight || 512;
  const scan = Math.min(1, MAX_SCAN / Math.max(naturalWidth, naturalHeight));
  const full = document.createElement("canvas");
  full.width = Math.max(1, Math.round(naturalWidth * scan));
  full.height = Math.max(1, Math.round(naturalHeight * scan));
  const fullContext = full.getContext("2d", { willReadFrequently: true });
  fullContext.drawImage(image, 0, 0, full.width, full.height);
  const bounds = opaqueBounds(fullContext, full.width, full.height);
  if (!bounds) throw new Error("图片是全透明的");

  const fit = Math.min(1, MAX_SIDE / Math.max(bounds.width, bounds.height));
  const output = document.createElement("canvas");
  output.width = Math.max(1, Math.round(bounds.width * fit));
  output.height = Math.max(1, Math.round(bounds.height * fit));
  const ctx = output.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(full, bounds.x, bounds.y, bounds.width, bounds.height, 0, 0, output.width, output.height);
  return output.toDataURL("image/png");
}
