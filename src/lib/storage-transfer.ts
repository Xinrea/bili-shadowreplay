import type { TaskRow } from "./db";
import type { StorageConfig } from "./interface";
import { storageBackend } from "./recording-storage";

export interface StorageTransferPlan {
  source_backend_id: string;
  target_backend_id: string;
  recording_count: number;
  total_bytes: number;
  required_bytes: number;
  available_bytes: number;
  cleanup_count: number;
  can_transfer: boolean;
  blockers: string[];
  /** Root-level prediction; EXDEV can require a checked copy fallback at execution. */
  transfer_mode: "rename" | "copy" | "mixed";
}

export interface StorageTransferResult {
  transferred_recordings: number;
  cleaned_recordings: number;
  failed_recording?: string | null;
  error?: string | null;
}

export function transferSelectionError(storage: StorageConfig, source: string, target: string): string {
  if (!source || !target) return "请选择来源和目标存储。";
  if (source === target) return "来源和目标不能是同一存储。";
  if (storageBackend(storage, source)?.type !== "local" || storageBackend(storage, target)?.type !== "local") {
    return "请选择两个已配置的本地存储；缺失或不受支持的存储不能转移。";
  }
  return "";
}

export function transferModeLabel(mode: StorageTransferPlan["transfer_mode"]): string {
  switch (mode) {
    case "rename": return "同文件系统，优先快速移动";
    case "mixed": return "快速移动与复制";
    case "copy": return "复制并校验后清理来源";
  }
}

export function transferResult(task: TaskRow): StorageTransferResult | null {
  try {
    const result = JSON.parse(task.metadata)?.result;
    return result && typeof result.transferred_recordings === "number" &&
      typeof result.cleaned_recordings === "number" ? result : null;
  } catch {
    return null;
  }
}

export function transferStatus(status: string): string {
  switch (status.toLowerCase()) {
    case "pending": return "排队中";
    case "processing": return "转移中";
    case "success":
    case "completed": return "已完成";
    case "error":
    case "failed": return "已停止，请检查后重试";
    default: return status;
  }
}

export function transferBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value.toFixed(1)} ${units[unit]}`;
}
