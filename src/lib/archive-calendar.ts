import type { RecordItem } from "./db";

const HOUR = 3_600_000;
// Reserve room for the viewport and ticks near JavaScript's date limits.
const DATE_LIMIT = 8.64e15 - 366 * 24 * HOUR;
export const ARCHIVE_CALENDAR_SPANS = [6, 12, 24, 72, 168, 720] as const;

export interface ArchiveCalendarEvent {
  archive: RecordItem;
  key: string;
  start: number;
  end: number;
  timingKnown: boolean;
  left: number;
  width: number;
  hitLeft: number;
  hitWidth: number;
  continuesBefore: boolean;
  continuesAfter: boolean;
  lane: number;
}

export function calendarDateKey(timestamp: number): string {
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function validTimestamp(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && Math.abs(value) < DATE_LIMIT;
}

function archiveStart(record: RecordItem): number {
  return validTimestamp(record.recording_started_at) ? record.recording_started_at : Date.parse(record.created_at);
}

export function latestArchiveDate(records: readonly RecordItem[]): number | null {
  let latest: number | null = null;
  for (const record of records) {
    const start = archiveStart(record);
    if (validTimestamp(start) && (latest === null || start > latest)) latest = start;
  }
  return latest;
}

/** One continuous elapsed-time axis. Events are never split at midnight or assigned list rows. */
export function buildArchiveCalendar(records: readonly RecordItem[], center: number, hours = 24, plotWidth = 960) {
  const width = Math.max(28, Math.min(16_000, Number.isFinite(plotWidth) ? plotWidth : 960));
  const spanHours = Number.isFinite(hours) && hours > 0 ? Math.min(8760, hours) : 24;
  const span = spanHours * HOUR;
  const anchor = validTimestamp(center) ? center : Date.now();
  const start = anchor - span / 2;
  const end = anchor + span / 2;
  const x = (time: number) => (time - start) / span * width;
  const events: ArchiveCalendarEvent[] = [];
  let undated = 0;
  for (const archive of records) {
    const from = archiveStart(archive);
    if (!validTimestamp(from)) { undated++; continue; }
    const timingKnown = validTimestamp(archive.recording_started_at) &&
      validTimestamp(archive.recording_ended_at) && archive.recording_ended_at >= from;
    // Media duration excludes ads and outages. Missing clock data is a marker,
    // rather than an invented wall-clock end time.
    const to = timingKnown ? archive.recording_ended_at! : from;
    if (from >= end || (to === from ? from < start : to <= start)) continue;
    const eventWidth = Math.min(width, Math.max(2, x(Math.min(end, to)) - x(Math.max(start, from))));
    const left = Math.min(width - eventWidth, Math.max(0, x(Math.max(start, from))));
    const hitWidth = Math.min(width, Math.max(28, eventWidth));
    const hitLeft = Math.max(0, Math.min(width - hitWidth, left - (hitWidth - eventWidth) / 2));
    events.push({ archive, key: `${archive.platform}:${archive.room_id}:${archive.live_id}`, start: from, end: to, timingKnown,
      left, width: eventWidth, hitLeft, hitWidth, continuesBefore: from < start, continuesAfter: to > end, lane: 0 });
  }
  // Reuse free lanes for unrelated events; even a large history needs only a few lanes.
  events.sort((a, b) => a.hitLeft - b.hitLeft || a.start - b.start || a.key.localeCompare(b.key));
  const laneEnds: number[] = [];
  for (const event of events) {
    let lane = laneEnds.findIndex(right => right + 8 <= event.hitLeft);
    if (lane < 0) lane = laneEnds.length;
    laneEnds[lane] = event.hitLeft + event.hitWidth;
    event.lane = lane;
  }

  const desired = span / Math.max(1, Math.floor(width / 110));
  const steps = [1 / 60, 5 / 60, .25, .5, 1, 2, 3, 6, 12, 24, 48, 168, 336, 720, 2160, 8760];
  const step = (steps.find(hours => hours * HOUR >= desired) ?? 8760) * HOUR;
  const offset = new Date(start).getTimezoneOffset() * 60_000;
  const times = [start];
  for (let time = Math.ceil((start - offset) / step) * step + offset, count = 0; time < end && count < 500; time += step, count++) {
    if (x(time) >= 78 && width - x(time) >= 78) times.push(time);
  }
  times.push(end);
  const ticks = times.map((time, index) => ({
    time, left: x(time), align: index === 0 ? "start" : index === times.length - 1 ? "end" : "center",
    label: step >= 24 * HOUR ? new Date(time).toLocaleDateString(undefined, { month: "numeric", day: "numeric" })
      : new Date(time).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false }),
    detail: step >= 24 * HOUR ? new Date(time).toLocaleDateString(undefined, { weekday: "short" })
      : new Date(time).toLocaleDateString(undefined, { month: "numeric", day: "numeric" }),
  }));
  return { start, end, width, spanHours, events, lanes: laneEnds.length, count: events.length, undated, ticks };
}
