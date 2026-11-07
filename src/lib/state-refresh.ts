import { listen } from "./invoker";

/** Serializes snapshots and remembers changes that arrive during a request. */
export function createStateRefresh(
  refresh: () => Promise<void>,
  delayMs = 200,
  onError: (error: unknown) => void = (error) => console.error("刷新数据失败:", error),
) {
  let stopped = false;
  let dirty = false;
  let pending: Promise<void> | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer() {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  async function drain() {
    try {
      while (dirty && !stopped) {
        dirty = false;
        try {
          await refresh();
        } catch (error) {
          onError(error);
        }
      }
    } finally {
      pending = null;
    }
  }

  function run(): Promise<void> {
    if (stopped) return Promise.resolve();
    clearTimer();
    dirty = true;
    if (!pending) {
      pending = Promise.resolve().then(drain);
    }
    return pending;
  }

  function schedule() {
    if (stopped) return;
    if (pending) {
      dirty = true;
    } else if (timer === null) {
      // A fixed coalescing window prevents continuous events postponing refresh.
      timer = setTimeout(() => { void run(); }, delayMs);
    }
  }

  return {
    run,
    schedule,
    stop() {
      stopped = true;
      dirty = false;
      clearTimer();
    },
  };
}

/** Subscribe before the first snapshot; return synchronous onMount cleanup. */
export function watchStateChanges(
  refresh: ReturnType<typeof createStateRefresh>,
  events: string[],
  fallbackMs = 30_000,
): () => void {
  let disposed = false;
  let ready = false;
  const unlisteners: Array<() => void> = [];
  const visible = () => document.visibilityState !== "hidden";
  const schedule = () => { if (ready && visible()) refresh.schedule(); };
  const resync = () => { if (ready && visible()) void refresh.run(); };

  void (async () => {
    await Promise.all([...new Set([...events, "state-resync"])].map(async (event) => {
      try {
        const unlisten = await listen(event, event === "state-resync" ? resync : schedule);
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      } catch (error) {
        console.error(`订阅 ${event} 失败:`, error);
      }
    }));
    if (!disposed) {
      ready = true;
      await refresh.run();
    }
  })();

  const timer = setInterval(schedule, fallbackMs);
  document.addEventListener("visibilitychange", resync);
  window.addEventListener("online", resync);

  return () => {
    disposed = true;
    ready = false;
    clearInterval(timer);
    refresh.stop();
    for (const unlisten of unlisteners) unlisten();
    document.removeEventListener("visibilitychange", resync);
    window.removeEventListener("online", resync);
  };
}
