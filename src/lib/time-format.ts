/**
 * Format a duration or position in seconds as `m:ss`, `h:mm:ss` or their
 * fractional variants (`m:ss.s`, ...). Rounds to the requested precision
 * BEFORE splitting into hours/minutes/seconds, so 59.96 s reads "1:00.0"
 * rather than the overflowed "0:60.0" that per-field `toFixed` produces.
 */
export function formatTimecode(seconds: number, decimals = 1): string {
  const factor = 10 ** decimals;
  const total = Math.round(Math.max(0, seconds) * factor) / factor;
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const secText =
    decimals > 0
      ? secs.toFixed(decimals).padStart(3 + decimals, "0")
      : String(secs).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${secText}`
    : `${minutes}:${secText}`;
}
