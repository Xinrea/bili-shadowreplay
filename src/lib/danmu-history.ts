import type { DanmuEntry } from "./interface";

export type DanmuHistoryState = "idle" | "loading" | "loaded" | "error";

function identity(entry: DanmuEntry): string {
  const emoji = entry.emoji_content;
  return JSON.stringify([
    entry.ts, entry.type, entry.content, entry.user_name ?? null,
    entry.price ?? null, entry.sc_duration ?? null,
    emoji?.url ?? null, emoji?.width ?? null, emoji?.height ?? null,
    emoji?.emoticon_unique ?? null,
  ]);
}

/** History and live delivery overlap. Preserve multiplicity within each source,
 * rather than dropping legitimate repeated messages with a Set. */
export function mergeDanmuHistory(history: DanmuEntry[], current: DanmuEntry[]): DanmuEntry[] {
  const remaining = new Map<string, number>();
  for (const entry of history) {
    const key = identity(entry);
    remaining.set(key, (remaining.get(key) ?? 0) + 1);
  }
  const merged = history.slice();
  for (const entry of current) {
    const key = identity(entry);
    const count = remaining.get(key) ?? 0;
    if (count > 0) remaining.set(key, count - 1);
    else merged.push(entry);
  }
  return merged.sort((a, b) => a.ts - b.ts);
}

/** History is independent of playback/live subscription. Failed requests retain
 * current events, can be retried, and can never update an unmounted player. */
export function createDanmuHistoryLoader(options: {
  load: () => Promise<DanmuEntry[]>;
  current: () => DanmuEntry[];
  flushLive: () => void;
  publish: (entries: DanmuEntry[]) => void;
  state: (state: DanmuHistoryState) => void;
  onError: (error: unknown) => void;
  signal: AbortSignal;
}) {
  let pending = false;
  async function reload() {
    if (pending || options.signal.aborted) return;
    pending = true;
    options.state("loading");
    try {
      const history = await options.load();
      if (options.signal.aborted) return;
      // Include arrivals still in the UI's batching timer before the merge.
      options.flushLive();
      options.publish(mergeDanmuHistory(history, options.current()));
      options.state("loaded");
    } catch (error) {
      if (options.signal.aborted) return;
      options.state("error");
      options.onError(error);
    } finally {
      pending = false;
    }
  }
  return { reload };
}
