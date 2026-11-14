/** Estimate bytes/s from recent archive-size snapshots, smoothing segment writes. */
export function createCacheGrowthEstimator(windowMs = 10_000) {
  let samples: Array<{ size: number; time: number }> = [];

  function reset() {
    samples = [];
  }

  return {
    reset,
    sample(size: number, time: number): number | null {
      if (!Number.isFinite(size) || size < 0 || !Number.isFinite(time)) {
        reset();
        return null;
      }

      const previous = samples[samples.length - 1];
      // Deletions and gaps in observation need a fresh baseline.
      if (previous && (size < previous.size || time <= previous.time || time - previous.time > windowMs)) {
        reset();
      }
      samples.push({ size, time });
      while (samples.length > 1 && samples[0].time < time - windowMs) {
        samples.shift();
      }

      const first = samples[0];
      const elapsed = (time - first.time) / 1000;
      return elapsed > 0 ? (size - first.size) / elapsed : null;
    },
  };
}
