// The rows of a list that are drawn when only those in view are: rows of one
// height, placed by index in a list as tall as all of them.

/**
 * Indexes of the rows to draw for the part of the list `top` to `top +
 * height` in view, `overscan` more each way, and `selected`, if any, so that
 * a row being edited stays. A view past the end, as after the list got
 * shorter and before its scroll follows, shows the last rows instead.
 */
export function shownRows(
  view: { top: number; height: number },
  rowHeight: number,
  overscan: number,
  length: number,
  selected = -1,
): number[] {
  const count = Math.ceil(view.height / rowHeight) + 2 * overscan;
  const first = Math.max(0, Math.min(Math.floor(view.top / rowHeight) - overscan, length - count));
  const last = Math.min(length, first + count);
  const rows: number[] = [];
  for (let index = first; index < last; index++) rows.push(index);
  if (selected >= 0 && selected < length && (selected < first || selected >= last)) rows.push(selected);
  return rows;
}
