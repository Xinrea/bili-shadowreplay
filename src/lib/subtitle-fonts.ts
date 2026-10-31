import { invoke } from "./invoker";

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
