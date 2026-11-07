import type { RoomCheckSample } from "./interface";

export const ROOM_CHECK_WINDOW_MS = 5 * 60 * 1000;

export interface RoomCheckBucket {
  key: number;
  first_checked_at: number;
  checked_at: number;
  duration_ms: number;
  error: string | null;
  count: number;
  failures: number;
  x: number;
  y: number;
  height: number;
  width: number;
  hitLeft: number;
  hitWidth: number;
}

/** Bin dense polls by time, keeping the longest duration and every failure count. */
export function buildRoomCheckTimeline(history: readonly RoomCheckSample[], now: number, width: number, coarse = false) {
  const valid = history.filter((sample) => Number.isFinite(sample.checked_at)
    && Number.isFinite(sample.duration_ms) && sample.duration_ms >= 0);
  const end = Math.max(now, ...valid.map((sample) => sample.checked_at));
  const start = end - ROOM_CHECK_WINDOW_MS;
  const samples = valid.filter((sample) => sample.checked_at >= start)
    .sort((a, b) => a.checked_at - b.checked_at);
  const left = 44, right = Math.max(left + 1, width - 12), top = 22, bottom = 94;
  const resultY = 114, labelY = 136, height = 148;
  const plotWidth = right - left;
  const slots = Math.max(1, Math.floor(plotWidth / (coarse ? 44 : 28)));
  const bucketMs = Math.ceil(ROOM_CHECK_WINDOW_MS / slots / 1000) * 1000;
  const buckets = new Map<number, RoomCheckBucket>();
  for (const sample of samples) {
    const key = Math.floor(sample.checked_at / bucketMs) * bucketMs;
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.checked_at = sample.checked_at;
      bucket.duration_ms = Math.max(bucket.duration_ms, sample.duration_ms);
      bucket.count++;
      if (sample.error !== null) { bucket.error = sample.error; bucket.failures++; }
    } else {
      buckets.set(key, {
        key, first_checked_at: sample.checked_at, checked_at: sample.checked_at,
        duration_ms: sample.duration_ms, error: sample.error, count: 1,
        failures: sample.error === null ? 0 : 1,
        x: 0, y: 0, height: 0, width: 0, hitLeft: 0, hitWidth: 0,
      });
    }
  }
  const max = Math.max(1, ...samples.map((sample) => sample.duration_ms));
  const step = 10 ** Math.floor(Math.log10(max));
  const ceiling = Math.ceil(max / step) * step;
  const unit = ceiling >= 2000 ? "s" : "ms";
  const scale = unit === "s" ? 1000 : 1;
  const x = (timestamp: number) => left + (timestamp - start) / ROOM_CHECK_WINDOW_MS * plotWidth;
  const bars = Array.from(buckets.values(), (bucket) => {
    const hitLeft = Math.min(right - 1, Math.max(left, x(bucket.key)));
    const hitRight = Math.max(hitLeft + 1, Math.min(right, x(bucket.key + bucketMs)));
    const hitWidth = Math.max(1, hitRight - hitLeft);
    const height = Math.max(2, bucket.duration_ms / ceiling * (bottom - top));
    return { ...bucket, x: hitLeft + hitWidth / 2, y: bottom - height, height,
      width: Math.max(1, Math.min(18, hitWidth * 0.55)), hitLeft, hitWidth };
  });
  return {
    bars, count: samples.length, failures: samples.filter((sample) => sample.error !== null).length,
    start, end, left, right, top, bottom, resultY, labelY, height, plotWidth, ceiling, unit,
    ticks: [0, ceiling / 2, ceiling].map((value) => ({
      y: bottom - value / ceiling * (bottom - top),
      label: Number((value / scale).toFixed(2)).toLocaleString(),
    })),
  };
}

export function roomCheckBucketLabel(bucket: RoomCheckBucket) {
  const timestamp = (value: number) => new Date(value).toLocaleTimeString();
  if (bucket.count === 1) {
    return `${timestamp(bucket.checked_at)}，${bucket.failures ? "检查失败" : "检查成功"}，耗时 ${bucket.duration_ms} ms${bucket.error !== null ? `，${bucket.error}` : ""}`;
  }
  return `${timestamp(bucket.first_checked_at)} 至 ${timestamp(bucket.checked_at)}，${bucket.count} 次检查，${bucket.failures} 次失败，最高耗时 ${bucket.duration_ms} ms${bucket.error !== null ? `，最近的错误：${bucket.error}` : ""}`;
}
