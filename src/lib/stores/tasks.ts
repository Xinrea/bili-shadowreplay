import { readable, writable } from "svelte/store";
import type { TaskRow } from "../db";
import type { ProgressUpdate } from "../interface";
import { invoke, listen } from "../invoker";
import { createStateRefresh, watchStateChanges } from "../state-refresh";
import { mergeActiveTasks, type ActiveTask } from "../task-progress";

/** Tasks started here that the backend has not listed yet. */
const announced = writable<TaskRow[]>([]);
/** Tasks cancelled from the task center, so whoever started them can stay silent. */
const cancelled = new Set<string>();
/** Settled announcements, keyed to the number of snapshots started before settling. */
const settled = new Map<string, number>();
let snapshotsStarted = 0;
/** Requests a backend snapshot while the feed runs. */
let snapshot: (() => Promise<void>) | null = null;

export const taskCenter = writable<{ open: boolean; highlight: string | null }>({
  open: false,
  highlight: null,
});

/**
 * Every pending or processing backend task, newest first, with live progress.
 * The feed runs while anything is subscribed.
 */
export const activeTasks = readable<ActiveTask[]>([], (set) => {
  let rows: TaskRow[] = [];
  let pending: TaskRow[] = [];
  const updates = new Map<string, ProgressUpdate>();
  const unlisteners = new Map<string, Promise<() => void>>();

  function publish() {
    set(mergeActiveTasks(rows, pending, updates));
  }

  // Progress events are scoped per task, so follow exactly the active ones.
  function followProgress() {
    const ids = new Set(mergeActiveTasks(rows, pending, updates).map((task) => task.id));
    for (const [id, unlisten] of unlisteners) {
      if (ids.has(id)) continue;
      void unlisten.then((stop) => stop());
      unlisteners.delete(id);
      updates.delete(id);
    }
    for (const id of ids) {
      if (unlisteners.has(id)) continue;
      unlisteners.set(
        id,
        listen(`progress-update:${id}`, (event) => {
          updates.set(id, event.payload);
          publish();
        }),
      );
    }
  }

  const refresh = createStateRefresh(async () => {
    const started = ++snapshotsStarted;
    const all = await invoke<TaskRow[]>("get_tasks");
    rows = all;
    const listed = new Set(all.map((task) => task.id));
    // Only a snapshot that began after settling can tell whether the backend
    // ever recorded the task.
    const isReplaced = (id: string) => listed.has(id) || (settled.get(id) ?? Infinity) < started;
    const replaced = new Set(pending.map((task) => task.id).filter(isReplaced));
    for (const id of settled.keys()) if (isReplaced(id)) settled.delete(id);
    if (replaced.size > 0) {
      announced.update((tasks) => tasks.filter((task) => !replaced.has(task.id)));
    }
    followProgress();
    publish();
  });
  snapshot = () => refresh.run();
  const stopWatching = watchStateChanges(refresh, ["tasks-changed"]);
  const stopAnnounced = announced.subscribe((tasks) => {
    pending = tasks;
    followProgress();
    publish();
  });

  return () => {
    snapshot = null;
    stopWatching();
    stopAnnounced();
    for (const unlisten of unlisteners.values()) void unlisten.then((stop) => stop());
    unlisteners.clear();
  };
});

/** Show a task immediately, before the backend has recorded it. */
export function announceTask(task: Pick<TaskRow, "id" | "task_type" | "metadata">) {
  announced.update((tasks) => [
    ...tasks,
    { ...task, status: "pending", message: "", created_at: new Date().toISOString() },
  ]);
}

/**
 * Forget local bookkeeping once the caller has the task's result. A task that
 * is still queued keeps showing until the backend lists it.
 */
export function settleTask(id: string) {
  cancelled.delete(id);
  if (!snapshot) {
    announced.update((tasks) => tasks.filter((task) => task.id !== id));
    return;
  }
  settled.set(id, snapshotsStarted);
  void snapshot();
}

export function wasCancelled(id: string) {
  return cancelled.has(id);
}

export async function cancelTask(id: string) {
  cancelled.add(id);
  try {
    await invoke("cancel", { eventId: id });
  } catch (error) {
    cancelled.delete(id);
    throw error;
  }
}

export function openTaskCenter(highlight: string | null = null) {
  taskCenter.set({ open: true, highlight });
}

export function closeTaskCenter() {
  taskCenter.set({ open: false, highlight: null });
}
