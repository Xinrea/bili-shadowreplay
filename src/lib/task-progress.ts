import type { TaskRow } from "./db";
import type { ProgressUpdate, StageProgress } from "./interface";

/** Whole completed percentage, or null while the stage length is unknown. */
export function progressPercent(progress?: StageProgress | null): number | null {
  if (!progress?.total || progress.total <= 0) return null;
  const fraction = Math.min(Math.max(progress.processed / progress.total, 0), 1);
  return Math.floor(fraction * 100);
}

/** Wall-clock seconds left at the current speed, or null without a total and speed. */
export function remainingSeconds(progress?: StageProgress | null): number | null {
  if (!progress?.total || !progress.speed || progress.speed <= 0) return null;
  return Math.max(progress.total - progress.processed, 0) / progress.speed;
}

export function formatClock(seconds: number, withHours = false) {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const parts = withHours || hours > 0 ? [hours, minutes, secs] : [minutes, secs];
  return parts.map((part) => part.toString().padStart(2, "0")).join(":");
}

export function formatRemaining(seconds: number) {
  // Round up first so 59.1 s reads "1 分钟" rather than "60 秒".
  const totalSeconds = Math.max(1, Math.ceil(seconds));
  if (totalSeconds < 60) return `${totalSeconds} 秒`;
  const totalMinutes = Math.ceil(totalSeconds / 60);
  if (totalMinutes < 60) return `${totalMinutes} 分钟`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return minutes > 0 ? `${hours} 小时 ${minutes} 分` : `${hours} 小时`;
}

const ACTIVE_STATUSES = new Set(["pending", "processing"]);

export interface ActiveTask extends TaskRow {
  /** Latest live progress, or null until the task reports any. */
  update: ProgressUpdate | null;
}

/**
 * Active backend tasks plus tasks started here that the backend has not listed
 * yet, newest first.
 */
export function mergeActiveTasks(
  rows: TaskRow[],
  announced: TaskRow[],
  updates: ReadonlyMap<string, ProgressUpdate>,
): ActiveTask[] {
  const listed = new Set(rows.map((row) => row.id));
  return [
    ...rows.filter((row) => ACTIVE_STATUSES.has(row.status.toLowerCase())),
    ...announced.filter((task) => !listed.has(task.id)),
  ]
    .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
    .map((task) => ({ ...task, update: updates.get(task.id) ?? null }));
}

export function taskTypeName(taskType: string) {
  switch (taskType.toLowerCase()) {
    case "clip_range":
      return "素材生成";
    case "generate_materials":
      return "素材生成";
    case "export_project":
      return "切片导出";
    case "render_material_danmu":
      return "素材压制弹幕";
    case "upload":
      return "投稿";
    case "upload_project":
      return "切片投稿";
    case "clip_video":
      return "切片裁剪";
    case "upload_procedure":
      return "切片投稿";
    case "generate_video_subtitle":
      return "生成字幕";
    case "encode_video_subtitle":
      return "压制字幕";
    case "generate_whole_clip":
      return "生成完整录播";
    case "generate_archive_summary":
      return "录播总结";
    default:
      return taskType;
  }
}

/** One-line summary of what a task works on, from its metadata. */
export function describeTask(task: Pick<TaskRow, "metadata">) {
  let metadata: any;
  try {
    metadata = JSON.parse(task.metadata);
  } catch {
    return "";
  }
  const params = metadata?.params;
  if (Array.isArray(params?.ranges)) {
    const ranges = params.ranges.filter((range: any) => range.activated !== false);
    const duration = ranges.reduce(
      (total: number, range: any) => total + Math.max(0, range.end - range.start),
      0,
    );
    const count = metadata.merge && ranges.length > 1
      ? `${ranges.length} 个选区合并为一个`
      : `${ranges.length} 个选区`;
    const parts = [params.title, count, formatClock(duration)];
    return parts.filter(Boolean).join(" · ");
  }
  if (typeof metadata?.clip_title === "string") {
    const duration =
      typeof metadata.start_time === "number" && typeof metadata.end_time === "number"
        ? formatClock(metadata.end_time - metadata.start_time)
        : "";
    return [metadata.clip_title, duration].filter(Boolean).join(" · ");
  }
  if (typeof metadata?.profile?.title === "string" && metadata.profile.title) {
    return metadata.profile.title;
  }
  if (typeof metadata?.title === "string" && metadata.title) return metadata.title;
  if (metadata?.project_id !== undefined) return `切片 #${metadata.project_id}`;
  if (metadata?.video_id !== undefined) return `素材 #${metadata.video_id}`;
  return "";
}

/** Task types the backend runs on its task queue; others cannot be cancelled. */
const CANCELLABLE_TASK_TYPES = new Set([
  "clip_range",
  "generate_materials",
  "export_project",
  "render_material_danmu",
  "clip_video",
  "encode_video_subtitle",
  "generate_whole_clip",
  "generate_archive_summary",
]);

export function isCancellableTask(taskType: string) {
  return CANCELLABLE_TASK_TYPES.has(taskType.toLowerCase());
}
