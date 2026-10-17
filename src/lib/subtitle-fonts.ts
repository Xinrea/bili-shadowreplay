import { get_static_url, invoke } from "./invoker";

interface PreviewFont {
  family: string;
  file: string;
  requested: boolean;
  /** The CJK fallback native libass is expected to choose on that machine. */
  fallback: boolean;
}

export interface SubtitleFonts {
  fonts: string[];
  fallbackFont: string;
  warning: string;
}

const requests = new Map<string, Promise<SubtitleFonts>>();

/** Load fonts from the machine doing the encoding, including in web mode. */
export function loadSubtitleFonts(fontName: string): Promise<SubtitleFonts> {
  const key = fontName.trim().toLowerCase();
  let pending = requests.get(key);
  if (!pending) {
    pending = (async () => {
      const result = await invoke<PreviewFont[]>("get_subtitle_preview_fonts", { fontName });
      if (!result.length) throw new Error("压制机器上未找到可用于字幕预览的字体");
      const urls = await Promise.all(result.map((font) => get_static_url("cache", font.file)));
      // A CJK fallback also contains Latin glyphs; use it for missing characters.
      const fallback = result.findIndex((font) => font.fallback);
      return {
        fonts: [...new Set(urls)],
        fallbackFont: urls[Math.max(fallback, 0)],
        warning: result.some((font) => font.requested)
          ? ""
          : `压制机器未安装「${fontName}」，预览使用替代字体；安装该字体后可提高一致性。`,
      };
    })().catch((error) => {
      requests.delete(key);
      throw error;
    });
    requests.set(key, pending);
  }
  return pending;
}

export interface SubtitleFontFamily {
  /** Name written to the ASS style. */
  family: string;
  /** Chinese family name, when the font provides one. */
  localized: string | null;
  /** Whether the font itself contains common Chinese characters. */
  chinese: boolean;
}

let families: Promise<SubtitleFontFamily[]> | null = null;

/** Installed families on the machine doing the encoding, loaded once. */
export function loadSubtitleFontFamilies(): Promise<SubtitleFontFamily[]> {
  families ??= invoke<SubtitleFontFamily[]>("get_subtitle_font_families").catch((error) => {
    families = null;
    throw error;
  });
  return families;
}
