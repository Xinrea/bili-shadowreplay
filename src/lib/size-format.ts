/**
 * Format a byte count with binary units: whole bytes below 1 KiB, then KiB,
 * MiB or GiB with two decimals (`512 B`, `1.50 KiB`, `2.00 GiB`).
 */
export function formatSize(size: number): string {
  if (size < 1024) {
    return `${size} B`;
  } else if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(2)} KiB`;
  } else if (size < 1024 * 1024 * 1024) {
    return `${(size / 1024 / 1024).toFixed(2)} MiB`;
  } else {
    return `${(size / 1024 / 1024 / 1024).toFixed(2)} GiB`;
  }
}
