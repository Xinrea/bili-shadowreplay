export type HeatLevel = "normal" | "extension" | "core";

export type HeatPoint = {
  time: number;
  count: number;
  level: HeatLevel;
};

export type HeatBarLayout = {
  left: number;
  width: number;
  height: number;
  level: HeatLevel;
};

export const MAX_VISIBLE_HEAT_BARS = 240;
export const DANMU_FLUSH_INTERVAL_MS = 500;
export const PEAK_DETECT_INTERVAL_MS = 1000;

type TimerHandle = ReturnType<typeof setTimeout> | number;

export type ThrottleClock = {
  now: () => number;
  schedule: (callback: () => void, delayMs: number) => TimerHandle;
  cancel: (handle: TimerHandle) => void;
};

function defaultClock(): ThrottleClock {
  return {
    now: () => Date.now(),
    schedule: (callback, delayMs) => setTimeout(callback, delayMs),
    cancel: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  };
}

export function createDanmuBatcher<T>(
  flush: (batch: T[]) => void,
  intervalMs = DANMU_FLUSH_INTERVAL_MS,
  clock: ThrottleClock = defaultClock(),
) {
  let pending: T[] = [];
  let timer: TimerHandle | null = null;

  const flushPending = () => {
    timer = null;
    if (pending.length === 0) return;
    const batch = pending;
    pending = [];
    flush(batch);
  };

  return {
    push(item: T) {
      pending.push(item);
      if (timer != null) return;
      timer = clock.schedule(flushPending, intervalMs);
    },
    flushNow() {
      if (timer != null) {
        clock.cancel(timer);
        timer = null;
      }
      flushPending();
    },
    cancel() {
      if (timer != null) {
        clock.cancel(timer);
        timer = null;
      }
      pending = [];
    },
  };
}

export type ThrottledRunner = (() => void) & { cancel: () => void };

export function createThrottledRunner(
  fn: () => void,
  intervalMs: number,
  clock: ThrottleClock = defaultClock(),
): ThrottledRunner {
  let last = 0;
  let timer: TimerHandle | null = null;

  const run = (() => {
    const now = clock.now();
    const remaining = intervalMs - (now - last);
    if (remaining <= 0) {
      last = now;
      fn();
      return;
    }
    if (timer != null) return;
    timer = clock.schedule(() => {
      timer = null;
      last = clock.now();
      fn();
    }, remaining);
  }) as ThrottledRunner;

  run.cancel = () => {
    if (timer == null) return;
    clock.cancel(timer);
    timer = null;
  };

  return run;
}

function clampPercent(
  seconds: number,
  timelineStart: number,
  timelineDuration: number,
) {
  if (timelineDuration <= 0) return 0;
  return Math.min(
    100,
    Math.max(0, ((seconds - timelineStart) / timelineDuration) * 100),
  );
}

export function layoutHeatBars(
  points: HeatPoint[],
  timelineStart: number,
  timelineDuration: number,
  maxHeat: number,
): HeatBarLayout[] {
  return points.map((point, index) => {
    const neighbor = points[index + 1] ?? points[index - 1] ?? point;
    const gap = Math.abs(neighbor.time - point.time);
    const left = clampPercent(point.time - gap / 2, timelineStart, timelineDuration);
    const width =
      timelineDuration <= 0
        ? 0.15
        : Math.max(0.15, Math.min((gap / timelineDuration) * 100, 100 - left));
    const height = maxHeat <= 0 ? 0 : Math.max(3, (point.count / maxHeat) * 26);
    return { left, width, height, level: point.level };
  });
}

function heatRank(level: HeatLevel) {
  if (level === "core") return 2;
  if (level === "extension") return 1;
  return 0;
}

export function downsampleHeatPoints(
  points: HeatPoint[],
  maxBars = MAX_VISIBLE_HEAT_BARS,
): HeatPoint[] {
  if (points.length <= maxBars) return points;
  const sampled: HeatPoint[] = [];
  const stride = points.length / maxBars;
  for (let i = 0; i < maxBars; i++) {
    const start = Math.floor(i * stride);
    const end = Math.max(start + 1, Math.floor((i + 1) * stride));
    let best = points[start];
    for (let j = start + 1; j < end && j < points.length; j++) {
      const candidate = points[j];
      if (
        candidate.count > best.count ||
        (candidate.count === best.count && heatRank(candidate.level) > heatRank(best.level))
      ) {
        best = candidate;
      }
    }
    sampled.push(best);
  }
  return sampled;
}

export function bucketDanmuStatistics(
  records: { ts: number; content: string }[],
  options: {
    localOffsetSec: number;
    globalOffsetSec: number;
    gapSec: number;
    filter?: string;
  },
): { ts: number; count: number }[] {
  const gapMs = Math.max(1, options.gapSec) * 1000;
  const counts: Record<number, number> = {};
  for (const entry of records) {
    if (options.filter && !entry.content.includes(options.filter)) continue;
    const timestamp =
      entry.ts + options.localOffsetSec * 1000 - options.globalOffsetSec * 1000;
    if (timestamp < 0) continue;
    const timeSlot = timestamp - (timestamp % gapMs);
    counts[timeSlot] = (counts[timeSlot] || 0) + 1;
  }
  return Object.keys(counts)
    .map((ts) => ({ ts: Number(ts), count: counts[Number(ts)] }))
    .sort((a, b) => a.ts - b.ts);
}

export function danmuStatisticsSignature(
  points: { ts: number; count: number }[],
): string {
  let signature = "";
  for (const point of points) {
    signature += `${point.ts}:${point.count},`;
  }
  return signature;
}

export function virtualListWindow(
  scrollTop: number,
  viewportHeight: number,
  itemHeight: number,
  count: number,
  buffer: number,
) {
  const safeCount = Math.max(0, count);
  const safeHeight = Math.max(1, itemHeight);
  const start = Math.max(0, Math.floor(Math.max(0, scrollTop) / safeHeight) - buffer);
  const end = Math.min(
    safeCount,
    Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / safeHeight) +
      buffer,
  );
  return {
    start,
    end,
    offsetY: start * safeHeight,
    totalHeight: safeCount * safeHeight,
  };
}
