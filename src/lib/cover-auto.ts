// AI 封面建议到封面元素的换算，无 DOM 访问以便测试；调用方负责量文字。
//
// 建议只决定画面帧、文字样式与摆放位置：文字内容就是视频标题原文，过长时
// 自动换行并缩小字号。元素放在 16:9 封面的元素空间里，4:3 封面显示其中部，
// 所以文字被钳制在中部宽度内，两种比例都完整可见。

import {
  clamp,
  COVER_SIZES,
  ELEMENT_SPACE,
  layoutText,
  styleOf,
  textFont,
  TEXT_STYLES,
  type CoverElement,
  type Measure,
  type TextElement,
  type TextStyle,
} from "./cover-elements";

/** 文字可占的宽度：4:3 封面中部宽 1440，两侧各留一点余量。 */
export const AUTO_COVER_MAX_WIDTH = 1360;
export const AUTO_COVER_MAX_HEIGHT = 700;
export const AUTO_COVER_MAX_LINES = 3;
export const AUTO_COVER_START_FONT_SIZE = 150;
export const AUTO_COVER_MIN_FONT_SIZE = 40;
export const AUTO_COVER_MARGIN = 24;

/** `id` 对应的样式；未知 id（旧版本后端或模型编造）回退到醒目黄。 */
export function coverStyleOf(id: string): TextStyle {
  return styleOf(TEXT_STYLES.find((style) => style.id === id) ?? TEXT_STYLES[0]);
}

/** 标题在 `fontSize` 下按宽度贪心换行；按码点切分，emoji 不会被劈开。 */
function wrapLines(title: string, fontSize: number, maxWidth: number, measure: Measure): string[] {
  const font = textFont({ font: "", fontSize, bold: true });
  const lines: string[] = [];
  let line = "";
  for (const char of title) {
    const next = line + char;
    if (line && measure(next, font) > maxWidth) {
      lines.push(line);
      line = char;
    } else {
      line = next;
    }
  }
  if (line) lines.push(line);
  return lines;
}

function titleElement(content: string, fontSize: number, style: TextStyle): TextElement {
  return {
    kind: "text",
    id: 0,
    x: ELEMENT_SPACE.width / 2,
    y: ELEMENT_SPACE.height / 2,
    rotation: 0,
    content,
    font: "",
    fontSize,
    bold: true,
    align: "center",
    ...style,
  };
}

/** The largest font size whose wrapped title fits the width, line and height caps. */
function fitTitle(
  title: string,
  style: TextStyle,
  measure: Measure,
): { content: string; fontSize: number } {
  for (
    let fontSize = AUTO_COVER_START_FONT_SIZE;
    fontSize >= AUTO_COVER_MIN_FONT_SIZE;
    fontSize -= 2
  ) {
    // Wrap under a width budget that shrinks by each round's measured
    // overflow, so the outline or background box fits too.
    let budget = AUTO_COVER_MAX_WIDTH;
    for (let round = 0; round < 4; round++) {
      const lines = wrapLines(title, fontSize, budget, measure);
      if (lines.length > AUTO_COVER_MAX_LINES) break;
      const layout = layoutText(titleElement(lines.join("\n"), fontSize, style), measure);
      if (layout.width <= AUTO_COVER_MAX_WIDTH && layout.height <= AUTO_COVER_MAX_HEIGHT) {
        return { content: lines.join("\n"), fontSize };
      }
      budget = Math.max(fontSize, budget - Math.ceil(layout.width - AUTO_COVER_MAX_WIDTH));
    }
  }
  // 最小字号下不再受行数限制：宁可多几行也不丢标题文字。
  const lines = wrapLines(title, AUTO_COVER_MIN_FONT_SIZE, AUTO_COVER_MAX_WIDTH, measure);
  return { content: lines.join("\n"), fontSize: AUTO_COVER_MIN_FONT_SIZE };
}

/** 把标题按 AI 建议的样式与位置排成封面元素；空白标题不产生元素。 */
export function buildAutoCoverElements(
  title: string,
  styleId: string,
  position: { x: number; y: number },
  measure: Measure,
): CoverElement[] {
  const text = title.trim();
  if (!text) return [];
  const style = coverStyleOf(styleId);
  const { content, fontSize } = fitTitle(text, style, measure);
  const element = titleElement(content, fontSize, style);
  // 位置钳制到文字完整可见的范围：横向限在中部 4:3 内，纵向留出上下边。
  const { width, height } = layoutText(element, measure);
  const home = COVER_SIZES.home;
  const xEdge = (ELEMENT_SPACE.width - home.width) / 2 + AUTO_COVER_MARGIN;
  element.x = clamp(
    position.x * ELEMENT_SPACE.width,
    xEdge + width / 2,
    ELEMENT_SPACE.width - xEdge - width / 2,
  );
  element.y = clamp(
    position.y * ELEMENT_SPACE.height,
    AUTO_COVER_MARGIN + height / 2,
    ELEMENT_SPACE.height - AUTO_COVER_MARGIN - height / 2,
  );
  return [element];
}
