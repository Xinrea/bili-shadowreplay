// When the player hands recorded danmaku over, step by step as playback goes:
// which stretch of the danmaku timeline is due, and when the screen starts
// over. Times are in milliseconds of the danmaku timeline.

/** How often the player steps. */
export const FEED_TICK_MS = 200;
/** Playback moving further than this between steps jumped, like a seek. */
const JUMP_MS = 2000;
/** After a jump, how far back danmaku may still be on screen. */
const CATCH_UP_MS = 20_000;

export interface FeedInput {
  /** Where playback is. */
  now: number;
  /** The danmaku offset; a change moves the timeline. */
  offset: number;
  paused: boolean;
  /** Whether live danmaku are shown as they arrive instead. */
  atLiveEdge: boolean;
}

export interface FeedStep {
  /** Whether what is on screen goes first. */
  clear: boolean;
  /** Whether live danmaku are drawn as they arrive, from now on. */
  live: boolean;
  /** Recorded danmaku due from here up to `now`, or null for none. */
  from: number | null;
}

export function createDanmakuFeed() {
  let cursor: number | undefined;
  let offset: number | undefined;
  let liveEdge: boolean | undefined;

  return {
    /** Forget where playback was, as after a seek: the next step catches up. */
    reset() {
      cursor = undefined;
    },
    /**
     * Playing on, the danmaku since the last step are due. A jump (a seek, a
     * changed offset, moving back or far ahead), even while paused, starts
     * over with the danmaku still on screen there. Crossing the live edge
     * starts over too, as live and recorded danmaku are laid out apart.
     */
    step(input: FeedInput): FeedStep {
      const crossed = liveEdge !== undefined && liveEdge !== input.atLiveEdge;
      liveEdge = input.atLiveEdge;
      if (input.atLiveEdge) {
        cursor = undefined;
        return { clear: crossed, live: true, from: null };
      }
      const jumped =
        cursor === undefined ||
        input.offset !== offset ||
        input.now < cursor ||
        input.now - cursor > JUMP_MS;
      if (input.paused && !jumped) return { clear: false, live: false, from: null };
      const from = jumped ? input.now - CATCH_UP_MS : (cursor as number);
      cursor = input.now;
      offset = input.offset;
      return { clear: crossed || jumped, live: false, from };
    },
  };
}
