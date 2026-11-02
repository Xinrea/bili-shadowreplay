import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/archive-calendar.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
await module.link(() => { throw new Error("Calendar geometry has no runtime dependencies"); });
await module.evaluate();
const { buildArchiveCalendar, calendarDateKey, latestArchiveDate } = module.namespace;
const HOUR = 3_600_000;
const at = (day, hour = 0, minute = 0) => new Date(2026, 10, day, hour, minute).getTime();
const record = (id, start, length, changes = {}) => ({ platform: "bilibili", room_id: "123", live_id: id, parent_id: id, title: id, created_at: new Date(start).toISOString(), length, size: 1000, cover: null,
  recording_started_at: start, recording_ended_at: start + (Number.isFinite(length) && length > 0 ? length * 1000 : 0), ...changes });

test("actual segment timestamps preserve skipped ads and override archive creation time", () => {
  const archive = record("twitch", at(2, 10), 50 * 60, {
    platform: "twitch", recording_started_at: at(1, 10), recording_ended_at: at(1, 11),
  });
  const layout = buildArchiveCalendar([archive], at(1, 10, 30), 6, 1200);
  assert.equal(layout.events.length, 1);
  assert.equal(layout.events[0].start, at(1, 10));
  assert.equal(layout.events[0].end, at(1, 11));
  assert.equal(layout.events[0].width, 200);
  assert.equal(layout.events[0].timingKnown, true);
  assert.equal(latestArchiveDate([archive]), at(1, 10));
});

test("missing or invalid segment times remain selectable without inventing an end from media duration", () => {
  const rows = [
    record("legacy", at(1, 10), 3600, { recording_started_at: null, recording_ended_at: null }),
    record("invalid", at(1, 11), 3600, { recording_ended_at: at(1, 10) }),
  ];
  const layout = buildArchiveCalendar(rows, at(1, 12), 12, 1200);
  assert.equal(layout.events.length, 2);
  assert.ok(layout.events.every(event => !event.timingKnown && event.end === event.start && event.width === 2));
});

test("cross-midnight recordings remain a single continuous event with proportional coverage", () => {
  const archive = record("overnight", at(1, 23), 3 * 3600);
  const layout = buildArchiveCalendar([archive], at(2), 12, 1200);
  assert.equal(layout.events.length, 1);
  assert.equal(layout.events[0].start, at(1, 23));
  assert.equal(layout.events[0].end, at(2, 2));
  assert.equal(layout.events[0].width, 300);
  assert.equal(layout.events[0].left, 500);
  assert.equal(layout.events[0].continuesBefore, false);
  assert.equal(layout.events[0].continuesAfter, false);
});

test("unrelated events reuse lanes while overlapping recordings are stacked", () => {
  const rows = [record("one", at(1, 10), 7200), record("overlap", at(1, 11), 7200), record("later", at(1, 15), 3600)];
  const before = structuredClone(rows);
  const layout = buildArchiveCalendar(rows, at(1, 12), 24, 1200);
  assert.equal(layout.count, 3);
  assert.equal(layout.lanes, 2);
  assert.equal(layout.events[0].lane, layout.events[2].lane);
  assert.notEqual(layout.events[0].lane, layout.events[1].lane);
  assert.deepEqual(rows, before);
});

test("long recordings are clipped at the viewport edges without splitting their identity or timestamps", () => {
  const archive = record("long", at(-2), 10 * 24 * 3600);
  const layout = buildArchiveCalendar([archive], at(1, 12), 24, 960);
  assert.equal(layout.count, 1);
  const event = layout.events[0];
  assert.equal(event.width, 960);
  assert.equal(event.left, 0);
  assert.equal(event.start, at(-2));
  assert.equal(event.end, at(8));
  assert.equal(event.continuesBefore, true);
  assert.equal(event.continuesAfter, true);
});

test("zoom changes the time scale and panning reveals other dates without list pagination", () => {
  const rows = Array.from({ length: 180 }, (_, i) => record(String(i), at(1, 10) - i * 12 * HOUR, 3600));
  const close = buildArchiveCalendar(rows, at(1, 12), 24, 1200);
  const wide = buildArchiveCalendar(rows, at(1, 12), 720, 1200);
  assert.ok(wide.count > 20);
  assert.equal(close.events.find(e => e.archive.live_id === "0").width, 50);
  assert.equal(wide.events.find(e => e.archive.live_id === "0").width, 2);
  assert.ok(wide.lanes < wide.count);
  const earlier = buildArchiveCalendar(rows, at(-30, 12), 24, 1200);
  assert.equal(earlier.events.some(e => e.archive.live_id === "0"), false);
  assert.ok(earlier.count > 0);
});

test("events at the range boundary use start-inclusive and end-exclusive semantics", () => {
  const rows = [record("ends-here", at(1, 11), 3600), record("starts-here", at(1, 12), 3600), record("ends-after", at(1, 23), 7200), record("next", at(2), 3600)];
  const layout = buildArchiveCalendar(rows, at(1, 18), 12, 1000);
  assert.deepEqual(Array.from(layout.events, e => e.archive.live_id), ["starts-here", "ends-after"]);
  assert.equal(layout.events[1].continuesAfter, true);
});

test("short events keep accurate bars and bounded clickable targets without overlapping hit regions in a lane", () => {
  const layout = buildArchiveCalendar([record("short", at(1, 10), 1), record("nearby", at(1, 10, 1), 1), record("edge", at(1, 23, 59), 1)], at(1, 12), 24, 960);
  for (const event of layout.events) {
    assert.equal(event.width, 2);
    assert.ok(event.hitWidth >= 28);
    assert.ok(event.hitLeft >= 0 && event.hitLeft + event.hitWidth <= layout.width);
  }
  assert.notEqual(layout.events[0].lane, layout.events[1].lane);
});

test("missing dates are reported, unknown durations stay selectable, and identity includes platform and room", () => {
  const rows = [record("bad", at(1), 3600, { created_at: "invalid", recording_started_at: null, recording_ended_at: null }), record("same", at(1, 10), NaN), record("same", at(1, 10), -1, { platform: "douyin" })];
  const layout = buildArchiveCalendar(rows, at(1, 12), 24, 960);
  assert.equal(layout.undated, 1);
  assert.equal(layout.count, 2);
  assert.ok(layout.events.every(event => event.start === event.end && event.width === 2));
  assert.equal(new Set(layout.events.map(event => event.key)).size, 2);
  assert.equal(latestArchiveDate([rows[0]]), null);
  assert.equal(latestArchiveDate(rows), at(1, 10));
  assert.equal(calendarDateKey(at(1, 23)), "2026-11-01");
});

test("a continuous axis measures actual elapsed time across daylight-saving changes", () => {
  const previous = process.env.TZ;
  try {
    process.env.TZ = "America/New_York";
    const start = new Date(2026, 10, 1, 0, 30).getTime();
    const layout = buildArchiveCalendar([record("dst", start, 4 * 3600)], start + 2 * HOUR, 12, 960);
    assert.equal(layout.events.length, 1);
    assert.equal(layout.events[0].width, 320);
    assert.equal(layout.events[0].end - layout.events[0].start, 4 * HOUR);
    assert.equal(new Date(layout.events[0].end).getHours(), 3);
    for (let i = 1; i < layout.ticks.length; i++) assert.ok(layout.ticks[i].time > layout.ticks[i - 1].time);
  } finally {
    if (previous === undefined) delete process.env.TZ;
    else process.env.TZ = previous;
  }
});

test("invalid scales and very long durations cannot create unbounded axes or tick counts", () => {
  const layout = buildArchiveCalendar([record("huge", at(1), 1e30)], at(1, 12), NaN, Infinity);
  assert.equal(layout.spanHours, 24);
  assert.ok(Number.isFinite(layout.width));
  assert.ok(layout.ticks.length < 100);
  assert.ok(layout.ticks.every(t => Number.isFinite(t.left) && t.left >= 0 && t.left <= layout.width));
});

test("latest archive, events and viewport share the safe date boundary", () => {
  const limit = 8.64e15 - 366 * 24 * HOUR;
  for (const timestamp of [0, 1 - limit, limit - 1]) {
    const archive = record("edge", timestamp, 0);
    const layout = buildArchiveCalendar([archive], timestamp, 8760);
    assert.equal(latestArchiveDate([archive]), timestamp);
    assert.equal(layout.events[0].start, timestamp);
    assert.equal((layout.start + layout.end) / 2, timestamp);
    assert.ok(layout.ticks.every(tick => Number.isFinite(new Date(tick.time).getTime())));
  }
  for (const timestamp of [NaN, Infinity, -Infinity, -limit, limit, -limit - 1, limit + 1]) {
    const archive = record("invalid", at(1), 0, {
      recording_started_at: timestamp,
      created_at: Number.isFinite(timestamp) ? new Date(timestamp).toISOString() : "invalid",
    });
    const before = Date.now();
    const layout = buildArchiveCalendar([archive], timestamp);
    const center = (layout.start + layout.end) / 2;
    assert.equal(latestArchiveDate([archive]), null);
    assert.equal(layout.count, 0);
    assert.equal(layout.undated, 1);
    assert.ok(center >= before && center <= Date.now());
  }
});
