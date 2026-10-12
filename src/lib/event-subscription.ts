/** Keep subscription setup independent of UI initialization and retry failures. */
export function keepEventSubscription(
  subscribe: (signal: AbortSignal) => Promise<() => void>,
  signal: AbortSignal,
  onError: (error: unknown) => void,
  retryMs = 3_000,
): () => void {
  const controller = new AbortController();
  let stopped = false;
  let unlisten: (() => void) | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function stop() {
    if (stopped) return;
    stopped = true;
    controller.abort();
    if (timer !== undefined) clearTimeout(timer);
    unlisten?.();
    signal.removeEventListener("abort", stop);
  }

  async function attempt() {
    try {
      const cleanup = await subscribe(controller.signal);
      if (stopped || signal.aborted) cleanup();
      else unlisten = cleanup;
    } catch (error) {
      if (stopped || signal.aborted) return;
      onError(error);
      timer = setTimeout(() => {
        timer = undefined;
        void attempt();
      }, retryMs);
    }
  }

  signal.addEventListener("abort", stop, { once: true });
  if (signal.aborted) stop();
  else void attempt();
  return stop;
}
