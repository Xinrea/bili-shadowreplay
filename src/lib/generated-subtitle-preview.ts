/** Selection for buildSubtitleAss's static generated subset, NOT external ASS. */
export function parseGeneratedSubtitlePreview(content: string) {
  const lines = content.split("\n");
  const events = new Map<number, { start: number; end: number }>();
  const boundaries = new Set<number>();
  const timestamp = (value: string) => {
    const match = /^(\d+):([0-5]\d):([0-5]\d)\.(\d{2})$/.exec(value);
    if (!match) throw new Error("生成字幕的时间格式无效");
    const centiseconds = ((Number(match[1]) * 60 + Number(match[2])) * 60 + Number(match[3])) * 100 + Number(match[4]);
    if (!Number.isSafeInteger(centiseconds)) throw new Error("生成字幕的时间超出范围");
    return centiseconds / 100;
  };
  if (content && !content.includes("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text")) {
    throw new Error("原生预览仅支持应用生成的字幕");
  }
  lines.forEach((line, index) => {
    if (!line.startsWith("Dialogue: ")) return;
    // Only timing is inspected; never split/rebuild Text (which may contain
    // commas, escaped line breaks, formatting tags, or multi-codepoint emoji).
    const fields = line.slice(10).split(",", 4);
    if (fields.length !== 4) throw new Error("生成字幕的事件格式无效");
    const start = timestamp(fields[1]);
    const end = timestamp(fields[2]);
    if (end <= start) throw new Error("生成字幕的结束时间无效");
    events.set(index, { start, end });
    boundaries.add(start);
    boundaries.add(end);
  });
  const sorted = [...boundaries].sort((a, b) => a - b);
  let previousInterval = -1;
  let previous: string | null = null;
  return {
    /** Start-inclusive/end-exclusive, including seeks backwards and overlaps. */
    at(time: number): string | null {
      if (!Number.isFinite(time) || time < 0) throw new Error("字幕预览时间无效");
      let low = 0;
      let high = sorted.length;
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (sorted[middle] <= time) low = middle + 1;
        else high = middle;
      }
      if (low === previousInterval) return previous;
      previousInterval = low;
      let active = false;
      const selected = lines.filter((_, index) => {
        const event = events.get(index);
        if (!event) return true;
        const visible = event.start <= time && time < event.end;
        active ||= visible;
        return visible;
      });
      previous = active ? selected.join("\n") : null;
      return previous;
    },
  };
}

export interface GeneratedPreviewRequest {
  assContent: string;
  time: number;
  fontName: string;
}
export interface GeneratedPreviewState {
  image: string;
  loading: boolean;
  error: string;
}

/** One in-flight native render + one replaceable pending request per preview.
 * Static cues are keyed by their exact document and font, not playback time.
 */
export class GeneratedSubtitlePreviewQueue {
  private generation = 0;
  private key: string | null | undefined;
  private pending: { request: GeneratedPreviewRequest; key: string; generation: number } | null = null;
  private running = false;
  private disposed = false;
  private cache = new Map<string, string>();
  private cacheSize = 0;

  constructor(
    private render: (request: GeneratedPreviewRequest) => Promise<string>,
    private publish: (state: GeneratedPreviewState) => void,
  ) {}

  update(assContent: string | null, time: number, fontName: string, retry = false) {
    if (this.disposed) return;
    const key = assContent === null ? null : `${fontName}\0${assContent}`;
    if (!retry && key === this.key) return;
    this.key = key;
    const generation = ++this.generation;
    this.pending = null;
    if (key === null || assContent === null) {
      this.publish({ image: "", loading: false, error: "" });
      return;
    }
    const cached = this.cache.get(key);
    if (cached && !retry) {
      this.cache.delete(key);
      this.cache.set(key, cached);
      this.publish({ image: cached, loading: false, error: "" });
      return;
    }
    if (new TextEncoder().encode(assContent).length > 256 * 1024) {
      this.publish({ image: "", loading: false, error: "当前字幕过大，无法预览（上限 256 KiB）" });
      return;
    }
    this.publish({ image: "", loading: true, error: "" });
    this.pending = { request: { assContent, time, fontName }, key, generation };
    void this.drain();
  }

  fail(error: unknown) {
    this.key = undefined;
    ++this.generation;
    this.pending = null;
    this.publish({ image: "", loading: false, error: error instanceof Error ? error.message : String(error) });
  }

  private async drain() {
    if (this.running || this.disposed) return;
    this.running = true;
    try {
      while (this.pending && !this.disposed) {
        const job = this.pending;
        this.pending = null;
        try {
          const image = await this.render(job.request);
          if (this.disposed) return;
          const old = this.cache.get(job.key);
          if (old) this.cacheSize -= job.key.length + old.length;
          this.cache.delete(job.key);
          this.cache.set(job.key, image);
          this.cacheSize += job.key.length + image.length;
          // Bound both count and retained UTF-16 bytes, including document keys.
          while (this.cache.size > 8 || this.cacheSize * 2 > 8 * 1024 * 1024) {
            const first = this.cache.entries().next().value!;
            this.cache.delete(first[0]);
            this.cacheSize -= first[0].length + first[1].length;
          }
          if (job.generation === this.generation) this.publish({ image, loading: false, error: "" });
        } catch (error) {
          if (!this.disposed && job.generation === this.generation) {
            this.publish({ image: "", loading: false, error: `字幕预览失败：${error instanceof Error ? error.message : String(error)}` });
          }
        }
      }
    } finally {
      this.running = false;
    }
  }

  dispose() {
    this.disposed = true;
    ++this.generation;
    this.pending = null;
    this.cache.clear();
    this.cacheSize = 0;
  }
}
