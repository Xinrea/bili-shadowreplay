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

  /** Whether a message from `from` on overlaps `top..bottom`. */
  function overlaps(from: number, top: number, bottom: number) {
    const other = placed.slice(from).find((other) => other.bottom >= top);
    return other !== undefined && other.top <= bottom;
  }

  /**
   * Bilibili's space manager for scrolling messages: the first top, from the
   * top of the player down, where a message entering at `now` overlaps no
   * message on screen, or follows it closely enough never to catch up with
   * it. Nothing when it would go below the player, as the room drops those,
   * unless `keep`: then the top of the player.
   */
  function place(
    now: number,
    width: number,
    height: number,
    container: { width: number; height: number },
    keep = false,
  ): Flight | null {
    placed = placed.filter((other) => other.end > now);
    const entryX = container.width + 1 + SAFE_RIGHT;
    const speed = (container.width + width) / baseDuration(width, container.width);
    const travel = Math.ceil(entryX + width + SAFE_LEFT);
    const duration = Math.ceil((travel / speed) * 100) / 100;
    const middle = now + entryX / speed;
    const top = findTop(now, entryX, middle, height, container.height + 1) ?? (keep ? 0 : null);
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
      if (other.top > bottom) return top;
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
