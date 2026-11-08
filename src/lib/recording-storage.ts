import type { StorageConfig } from "./interface";
import type { RecordItem } from "./db";

const backendIds = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

/** Display order must not depend on backend map serialization or default selection. */
export function storageBackendEntries(storage: StorageConfig) {
  return Object.entries(storage.backends).sort(([left], [right]) =>
    backendIds.compare(left, right) || (left < right ? -1 : left > right ? 1 : 0));
}

/** Older servers omit availability; only an explicit false blocks file operations. */
export function archiveFilesAvailable(archive: RecordItem | null | undefined): boolean {
  return !!archive && archive.storage_available !== false;
}

export function archiveStorageReason(archive: RecordItem): string {
  return archive.storage_unavailable_reason ||
    `录播所属存储 ${archive.storage_backend_id || "（未知）"} 未配置或不受支持，无法访问文件。录播记录仍保留。`;
}

/** Only configured IDs are valid; never fall back for an unknown override. */
export function storageBackend(storage: StorageConfig, id: string) {
  return Object.prototype.hasOwnProperty.call(storage.backends, id)
    ? storage.backends[id]
    : undefined;
}
