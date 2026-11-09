// 把 AI 封面建议离屏渲染成两张封面 JPEG 数据 URL，与封面编辑器共用渲染管线。

import {
  aspectOf,
  COVER_SIZES,
  defaultFrame,
  elementArea,
  frameRect,
  sceneOf,
} from "../../cover-elements";
import { buildAutoCoverElements } from "../../cover-auto";
import type { CoverSuggestion } from "../../interface";
import { measure, prepareBackground, renderCover } from "./render";

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("封面画面加载失败"));
    image.src = src;
  });
}

/** 16:9 与 4:3 各渲染一张：画面帧铺满场景，标题文字按建议摆放。 */
export async function renderAutoCover(
  suggestion: CoverSuggestion,
  title: string,
): Promise<{ cover: string; cover43: string }> {
  const image = await loadImage(`data:image/jpeg;base64,${suggestion.frame_base64}`);
  const background = prepareBackground(image, image.naturalWidth, image.naturalHeight);
  const scene = sceneOf(image.naturalWidth, image.naturalHeight);
  const elements = buildAutoCoverElements(title, suggestion.style, suggestion.position, measure);
  const ratios = [
    ["cover", "wide"],
    ["cover43", "home"],
  ] as const;
  const exports = ratios.map(([key, ratio]) => {
    // Same export settings as the cover editor's `exportCover`.
    const canvas = document.createElement("canvas");
    canvas.width = COVER_SIZES[ratio].width;
    canvas.height = COVER_SIZES[ratio].height;
    const aspect = aspectOf(ratio);
    renderCover(
      canvas.getContext("2d"),
      scene,
      frameRect(scene, aspect, defaultFrame(scene, aspect)),
      background,
      elementArea(ratio),
      elements,
    );
    return [key, canvas.toDataURL("image/jpeg", 0.92)] as const;
  });
  return Object.fromEntries(exports) as { cover: string; cover43: string };
}
