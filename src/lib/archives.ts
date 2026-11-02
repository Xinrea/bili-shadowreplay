import type { RecordItem } from "./db";
import { get_static_url, invoke } from "./invoker";

export type ArchiveSortField = "created_at" | "title" | "length" | "size" | "room_id";
export type ArchiveSortOrder = "asc" | "desc";
export type ArchiveRoom = { platform: string; room_id: string };

export function archiveKey(record: ArchiveRoom & { live_id: string }): string {
  return `${record.platform}:${record.room_id}:${record.live_id}`;
}

/** Room totals use every archive once, including progress newer than the archive snapshot. */
export function summarizeRoomArchives(
  records: readonly RecordItem[],
  room: ArchiveRoom,
  currentRecord: RecordItem | null = null,
): { count: number; length: number; size: number } {
  const unique = new Map<string, { length: number; size: number }>();
  const nonnegative = (value: number) => Number.isFinite(value) && value > 0 ? value : 0;
  function include(record: RecordItem) {
    if (record.platform !== room.platform || record.room_id !== room.room_id) return;
    const key = archiveKey(record);
    const previous = unique.get(key);
    unique.set(key, {
      length: Math.max(previous?.length ?? 0, nonnegative(record.length)),
      size: Math.max(previous?.size ?? 0, nonnegative(record.size)),
    });
  }
  for (const record of records) include(record);
  if (currentRecord) include(currentRecord);
  let length = 0, size = 0;
  for (const record of unique.values()) { length += record.length; size += record.size; }
  return { count: unique.size, length, size };
}

/** Stable sorting for both the full archive page and an individual room. */
export function sortArchives(records: RecordItem[], field: ArchiveSortField, order: ArchiveSortOrder): RecordItem[] {
  const value = (record: RecordItem) => field === "created_at"
    ? Date.parse(record.created_at) || 0
    : field === "title" ? record.title.toLocaleLowerCase() : record[field];
  return [...records].sort((a, b) => {
    const left = value(a);
    const right = value(b);
    const comparison = left < right ? -1 : left > right ? 1 : 0;
    return (comparison || archiveKey(a).localeCompare(archiveKey(b))) * (order === "asc" ? 1 : -1);
  });
}

/** Load every page before local sorting/pagination so older recordings remain reachable. */
export async function loadRoomArchives(
  room: ArchiveRoom,
  active: () => boolean = () => true,
): Promise<RecordItem[]> {
  const pageSize = 100;
  const records = new Map<string, RecordItem>();
  for (let offset = 0; active(); offset += pageSize) {
    const page = await invoke<RecordItem[]>("get_archives", {
      platform: room.platform,
      roomId: room.room_id,
      offset,
      limit: pageSize,
    });
    if (!active()) return [];
    const before = records.size;
    for (const record of page) records.set(archiveKey(record), record);
    // Also stop if a server repeats a full page instead of honoring the offset.
    if (page.length < pageSize || records.size === before) break;
  }
  const result: RecordItem[] = [];
  for (const record of records.values()) {
    if (!active()) return [];
    result.push({
      ...record,
      cover: await get_static_url("cache", `${record.platform}/${record.room_id}/${record.live_id}/cover.jpg`),
    });
  }
  return result;
}
