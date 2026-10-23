// Bilibili's live-room danmaku layout, as burned into videos by
// src-tauri/src/danmu2ass.rs: the roll mode, its space manager and
// `getBaseDmDuration` of https://s1.hdslb.com/bfs/live-pkg/danmaku/danmaku-v2.js,
// and the room's render queue, with the room player's default settings (the
// whole player, normal density and speed). The font size is the danmaku
// style's, as burned into videos. Sizes are in CSS pixels of the player,
// times in seconds.

/** Row height, and image height, in font sizes. */
export const LINE_HEIGHT = 1.125;
/** Messages enter this far beyond the right edge, and leave this far beyond the left one. */
const SAFE_RIGHT = 50;
const SAFE_LEFT = 50;
/** Seconds a message must stay behind the one ahead of it in its row, at normal density. */
const FOLLOW_GAP = 0.1;
/** The room draws arriving messages through at most this many timers, each set this much later per timer already waiting... */
const MAX_TIMERS = 10;
const TIMER_STEP_MS = 100;
/** ...and keeps at most one more than this many waiting, dropping the oldest. */
const QUEUE_LIMIT = 20;

/** `getBaseDmDuration`: seconds for a message `width` wide to cross a player `container` wide. */
export function baseDuration(width: number, container: number): number {
  const fallback = 4.5;
  // [boundary, max, min]: players up to the next boundary use these.
  const steps = [
    [558, 13, 7], [888, 14, 8], [1082, 14, 9], [1246, 15, 10],
    [1452, 16, 11], [1744, 18, 13], [2314, 20, 14], [2560, 22, 16],
  ];
  if (!(container > 0) || !(width > 0) || container < steps[0][0]) return fallback;
  let max: number;
  let min: number;
  const index = steps.findIndex(([boundary], index) => index > 0 && container <= boundary);
  if (index > 0) {
    [, max, min] = steps[index - 1];
  } else {
    const [boundary, lastMax, lastMin] = steps[steps.length - 1];
    const extra = Math.floor((container - boundary) / 500);
    [max, min] = [lastMax + extra, lastMin + extra];
  }
  const midpoint = Math.log((max - min) / 0.1 - 1) / 0.2 + 0.2;
  return min + (max - min) / (1 + Math.exp(-0.2 * (width / container - midpoint))) || fallback;
}

/**
 * The em of 微软雅黑 within the line box an ASS font size sets, which burned
 * danmaku are laid out with (see danmu2ass.rs).
 */
const FONT_EM = 2048 / 2703;
/** Burned danmaku sizes are in units of a video this high. */
const PLAY_RES_Y = 720;

/**
 * The CSS font size, in a player `height` high, of danmaku styled `fontSize`:
 * the size they are burned at, in a video shown that high. The default, 36,
 * is about Bilibili's in a full-screen 1080p player.
 */
export function danmakuFontSize(fontSize: number, height: number): number {
  return (fontSize * FONT_EM * height) / PLAY_RES_Y;
}

/** Where a message flies: its left edge moves `travel` leftwards from `entryX` in `duration`. */
export interface Flight {
  top: number;
  entryX: number;
  travel: number;
  duration: number;
}

interface Placed {
  top: number;
  bottom: number;
  width: number;
  speed: number;
  /** When its left edge reaches the left of the player. */
  middle: number;
  end: number;
}

/** The messages on screen, by bottom, then by when they leave. */
export function createDanmakuSpace() {
  let placed: Placed[] = [];

  /**
   * Whether a message from `from` on overlaps `top..bottom`. Bilibili looks
   * at the first only, which misses taller ones (super chats, images) further
   * down the list.
   */
  function overlaps(from: number, top: number, bottom: number) {
    return placed.slice(from).some((other) => other.bottom >= top && other.top <= bottom);
  }

  /**
   * Bilibili's space manager for scrolling messages: the first top, from the
   * top of the player down, where a message entering at `now` overlaps no
   * message on screen, or follows it closely enough never to catch up with
   * it. Nothing when it would go below the player, as the room drops those.
   * With `force`, it flies over the top row without taking up space instead,
   * room or not.
   */
  function place(
    now: number,
    width: number,
    height: number,
    container: { width: number; height: number },
    force = false,
  ): Flight | null {
    const entryX = container.width + 1 + SAFE_RIGHT;
    const speed = (container.width + width) / baseDuration(width, container.width);
    const travel = Math.ceil(entryX + width + SAFE_LEFT);
    const duration = Math.ceil((travel / speed) * 100) / 100;
    if (force) return { top: 0, entryX, travel, duration };
    placed = placed.filter((other) => other.end > now);
    const middle = now + entryX / speed;
    const top = findTop(now, entryX, middle, height, container.height + 1);
    if (top === null) return null;
    const entry: Placed = {
      top,
      bottom: top + height,
      width,
      speed,
      middle,
      end: now + duration,
    };
    const at = placed.findIndex(
      (other) => other.bottom > entry.bottom || (other.bottom === entry.bottom && other.end > entry.end),
    );
    placed.splice(at < 0 ? placed.length : at, 0, entry);
    return { top, entryX, travel, duration };
  }

  function findTop(now: number, entryX: number, middle: number, height: number, limit: number) {
    let top = 0;
    let bottom = height;
    for (let index = 0; index < placed.length; index++) {
      const other = placed[index];
      if (bottom > limit) break;
      if (other.bottom <= top) continue;
      if (other.top > bottom) {
        if (!overlaps(index + 1, top, bottom)) return top;
        continue;
      }
      const otherX = (other.middle - now) * other.speed;
      if (entryX > otherX + other.width && middle > other.end + FOLLOW_GAP) {
        if (bottom <= other.bottom && !overlaps(index + 1, top, bottom)) return top;
        continue;
      }
      top = other.bottom + 1;
      bottom = top + height;
      if (bottom > limit) return null;
    }
    return bottom > limit ? null : top;
  }

  return {
    place,
    clear() {
      placed = [];
    },
  };
}

export type DanmakuSpace = ReturnType<typeof createDanmakuSpace>;

/** How long a message that must be kept waits for room before it flies over the top. */
export const KEEP_WAIT_S = 5;

/**
 * Where a message flies, asked once it is laid out, and with `keep`, every
 * frame while there is no room. One with a time of its own, `at` (recorded),
 * is placed then, already as far into its flight as `clock()` is, or not at
 * all; others enter at `clock()`. A kept one without room at its time tries
 * again from the next frame, entering then: after the others due with it
 * are placed at their own times, which it must not get in the way of. Once
 * it has waited KEEP_WAIT_S since its time, or its first try, it flies over
 * the top without taking up space, as far into that flight as it would be
 * had it entered when the wait was over.
 */
export function createPlacer(
  space: DanmakuSpace,
  clock: () => number,
  container: () => { width: number; height: number },
  { at, keep = false }: { at?: number; keep?: boolean } = {},
) {
  let since: number | undefined;
  return (width: number, height: number): (Flight & { elapsed: number }) | null => {
    const now = clock();
    if (since === undefined && at !== undefined) {
      since = at;
      const flight = space.place(at, width, height, container());
      return flight && { ...flight, elapsed: now - at };
    }
    since ??= now;
    if (keep && now - since >= KEEP_WAIT_S) {
      const start = since + KEEP_WAIT_S;
      const flight = space.place(start, width, height, container(), true);
      return flight && { ...flight, elapsed: now - start };
    }
    const flight = space.place(now, width, height, container());
    return flight && { ...flight, elapsed: 0 };
  };
}

/**
 * The room's queue (see createDanmakuPacer) run on another clock, such as a
 * video's: when each message arriving at `times` (ms) is drawn, or null when
 * it is dropped. Burned videos are paced so (see danmu2ass.rs).
 */
export function paceTimes(times: number[]): (number | null)[] {
  const order = times.map((_, index) => index).sort((a, b) => times[a] - times[b]);
  const drawn: (number | null)[] = times.map(() => null);
  const queue: number[] = [];
  // [when it fires, the message's index]
  const timers: [number, number][] = [];
  let next = 0;
  for (;;) {
    let timer = -1;
    for (let i = 0; i < timers.length; i++) {
      if (timer < 0 || timers[i][0] < timers[timer][0]) timer = i;
    }
    const arrival = next < order.length ? times[order[next]] : undefined;
    let now: number;
    // Messages arriving together are all queued before a timer fires.
    if (timer >= 0 && (arrival === undefined || timers[timer][0] < arrival)) {
      const [at, index] = timers.splice(timer, 1)[0];
      drawn[index] = at;
      now = at;
    } else if (arrival !== undefined) {
      if (queue.length > QUEUE_LIMIT) queue.shift();
      queue.push(order[next++]);
      now = arrival;
    } else {
      break;
    }
    while (timers.length < MAX_TIMERS && queue.length > 0) {
      timers.push([now + timers.length * TIMER_STEP_MS, queue.shift() as number]);
    }
  }
  return drawn;
}

/**
 * The room player doesn't draw a message the moment it arrives. It queues it,
 * and a timer draws it later by TIMER_STEP_MS for each timer already waiting:
 * a burst is spread out to about ten messages a second, and its oldest
 * messages are dropped while more keep arriving. A quiet stream is drawn at
 * once.
 */
export function createDanmakuPacer<T>(
  draw: (item: T) => void,
  timer: {
    set: (callback: () => void, delay: number) => unknown;
    clear: (id: unknown) => void;
  } = {
    set: (callback, delay) => setTimeout(callback, delay),
    clear: (id) => clearTimeout(id as ReturnType<typeof setTimeout>),
  },
) {
  const queue: T[] = [];
  const timers = new Set<unknown>();

  function pump() {
    while (timers.size < MAX_TIMERS && queue.length > 0) {
      const item = queue.shift() as T;
      const id = timer.set(() => {
        timers.delete(id);
        draw(item);
        pump();
      }, timers.size * TIMER_STEP_MS);
      timers.add(id);
    }
  }

  return {
    push(item: T) {
      if (queue.length > QUEUE_LIMIT) queue.shift();
      queue.push(item);
      pump();
    },
    clear() {
      queue.length = 0;
      for (const id of timers) timer.clear(id);
      timers.clear();
    },
  };
}
