/** PowerLive uses only the first hotwords it is sent. */
export const MAX_HOTWORDS = 500;
/** PowerLive rejects a longer context. */
export const MAX_CONTEXT_CHARS = 400;
/** The characters per subtitle line PowerLive accepts. */
export const SENTENCE_MAX_LENGTH = { min: 4, max: 50, default: 10 };

/** The nearest whole length PowerLive accepts, the default for an empty field. */
export function clampSentenceLength(value: number | null | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return SENTENCE_MAX_LENGTH.default;
  return Math.min(SENTENCE_MAX_LENGTH.max, Math.max(SENTENCE_MAX_LENGTH.min, Math.round(value)));
}

const SEPARATORS = /[,，、;；\n\r]+/;
const PUNCTUATION = /\p{P}/u;
const NON_ASCII = /[^\x00-\x7f]/;

/** Split typed or pasted text into hotwords, the way PowerLive does. */
export function splitHotwords(text: string): string[] {
  return text
    .split(SEPARATORS)
    .map((word) => word.trim().replace(/\s+/g, " "))
    .filter(Boolean);
}

/**
 * Why PowerLive would drop a hotword, or null when it is kept. The service
 * drops such words silently, so they are refused when typed.
 */
export function hotwordProblem(word: string): string | null {
  if (PUNCTUATION.test(word)) return "不能包含标点";
  if (NON_ASCII.test(word)) {
    return [...word].length > 15 ? "含中文时最多 15 个字" : null;
  }
  if (word.split(" ").length > 7) return "英文最多 7 个单词";
  if (word.length > 64) return "英文最多 64 个字符";
  return null;
}

export interface ContextSource {
  /** Display name of the live platform, empty when unknown. */
  platform: string;
  streamer: string;
  /** Titles of the streams the materials were cut from, most relevant first. */
  liveTitles: string[];
  clipTitle: string;
}

/**
 * A sentence about the recording for PowerLive: who streamed it and under
 * which titles, as these are the proper nouns most likely heard in it.
 */
export function buildSubtitleContext(source: ContextSource): string {
  const streamer = source.streamer.trim();
  const parts: string[] = [];
  if (streamer) parts.push(`${source.platform}主播${streamer}的直播`);
  else if (source.platform) parts.push(`${source.platform}直播录像`);
  const length = (extra: string) => [...`${[...parts, extra].join("，")}。`].length;
  const quote = (title: string) => `「${title}」`;
  const titles = [...new Set(source.liveTitles.map((title) => title.trim()).filter(Boolean))];
  let shown = "";
  // Whole titles only, within the limit.
  for (const title of titles) {
    if (length(`直播标题：${shown}${quote(title)}`) > MAX_CONTEXT_CHARS) break;
    shown += quote(title);
  }
  if (shown) parts.push(`直播标题：${shown}`);
  const clipTitle = source.clipTitle.trim();
  const clipPart = `切片标题：${quote(clipTitle)}`;
  if (clipTitle && !titles.includes(clipTitle) && length(clipPart) <= MAX_CONTEXT_CHARS) parts.push(clipPart);
  return parts.length ? [...`${parts.join("，")}。`].slice(0, MAX_CONTEXT_CHARS).join("") : "";
}
