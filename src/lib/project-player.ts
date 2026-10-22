// Seamless preview of a project timeline with two <video> elements: while one
// shows a clip, the other is already loaded at the next clip's in point, and
// they swap at the cut. The player itself behaves like a media element whose
// time is project time, so components that follow a <video> (the libass
// subtitle preview) can follow the whole project instead.
//
// As in an editing application, the sound is the clock. Every sound of the
// project, clips' own included, plays from a `PlaybackClock` (the preview's
// Web Audio), and the videos, always muted, only show pictures: they follow
// the clock as a `VideoFollower` keeps them, and one that falls behind loses
// frames, never the sound. Playing starts once the sound and the picture
// there are loaded, rather than starting and stuttering.
//
// While paused, an element still seeking is not sent elsewhere: a seek dropped
// halfway shows nothing, and when one follows the pointer every frame, as
// scrubbing does, few would land. It goes on to the last time asked once it
// lands.

export interface PlayerSegment {
  clipId: string;
  url: string;
  /** In and out points within the material. */
  start: number;
  end: number;
  /** Project time where the segment starts. */
  offset: number;
}

/** What plays the project's sound and tells the time it is at, while started. */
export interface PlaybackClock {
  /** Project time heard now; held where it started until the sound is heard. */
  readonly time: number;
  /** Load what is heard from project time `time`, to start there without a gap. */
  prepare(time: number): Promise<void>;
  start(time: number): void;
  stop(): void;
  /** Called while handling a user's input: browsers start sound only then. */
  resume?(): void;
  /** Load, without playing, what is heard from `time`, so that playing starts there at once. */
  preload?(time: number): void;
}

/** The parts of HTMLMediaElement that followers of a media clock read. */
export interface MediaClock extends EventTarget {
  readonly currentTime: number;
  readonly paused: boolean;
  readonly seeking: boolean;
  readonly readyState: number;
  readonly playbackRate: number;
}

// Played from the start again when asked to play this close to the end.
const SWITCH_EARLY = 1 / 60;
const TIME_UPDATE_INTERVAL_MS = 100;
/** Playing starts without the picture once it has waited this long for it. */
const READY_TIMEOUT_MS = 3000;
const READY_POLL_MS = 30;

// A seek of a playing video stops its picture until the frame is decoded from
// the keyframe before it, which in recordings with keyframes seconds apart
// takes WebKit long enough that the clock has moved on by more than a seek
// corrects: seeking on drift alone keeps it seeking, a slideshow. So a video
// is kept in step by playing a little faster or slower, and seeked only when
// far off, then to where the clock will be once the seek lands. Muted, neither
// is heard.
//
// Further off than `SEEK_DRIFT` a playing video is seeked. Further off than
// `CATCH_UP_DRIFT` it plays faster or slower, by `CATCH_UP_RATES` for each
// `CATCH_UP_STEP` off, until within `IN_STEP_DRIFT` again. Media elements tell
// their time in steps, which the two apart leave room for.
const SEEK_DRIFT = 1;
const PAUSED_DRIFT = 0.02;
const CATCH_UP_DRIFT = 0.04;
const IN_STEP_DRIFT = 0.01;
const CATCH_UP_STEP = 0.1;
const CATCH_UP_RATES = [0.05, 0.1, 0.2, 0.3];
/** How long a seek is taken to last before one has, and the most it is taken to. */
const FIRST_SEEK_TIME = 0.3;
const LONGEST_SEEK_TIME = 1.5;

/** Keeps a muted video showing what a clock is at. */
export class VideoFollower {
  private catchingUp = false;
  /** How long seeks of this element have lately taken, in seconds. */
  private seekTime = FIRST_SEEK_TIME;
  /** When the seek of ours going on was made, by `performance.now()`. */
  private seekStarted: number | null = null;
  private readonly seeked = () => {
    if (this.seekStarted === null) return;
    // The longer of the last and the one before, gently: one quick seek near
    // a keyframe says little about the next.
    const took = (performance.now() - this.seekStarted) / 1000;
    this.seekTime = Math.min(LONGEST_SEEK_TIME, Math.max(took, (this.seekTime + took) / 2));
    this.seekStarted = null;
  };

  constructor(private readonly element: HTMLVideoElement) {
    element.addEventListener("seeked", this.seeked);
  }

  dispose() {
    this.element.removeEventListener("seeked", this.seeked);
  }

  /** Play in step with the clock at media time `target`; at `last`, its last frame is held. */
  play(target: number, last = Number.POSITIVE_INFINITY) {
    const element = this.element;
    if (element.readyState < 1) return;
    const to = Math.min(target, last);
    if (to >= last) return this.rest(to);
    const drift = Math.abs(element.currentTime - to);
    if (drift > SEEK_DRIFT && !element.seeking) {
      this.rate(1);
      this.seek(Math.min(to + this.seekTime, last));
    } else if (drift > CATCH_UP_DRIFT || (this.catchingUp && drift > IN_STEP_DRIFT)) {
      const step = CATCH_UP_RATES[Math.min(CATCH_UP_RATES.length - 1, Math.floor(drift / CATCH_UP_STEP))];
      this.rate(element.currentTime > to ? 1 - step : 1 + step);
    } else {
      this.rate(1);
    }
    if (element.paused) element.play().catch(() => {});
  }

  /** Stop on the frame it shows. */
  hold() {
    this.rate(1);
    if (!this.element.paused) this.element.pause();
  }

  /** Stop on media time `target`. */
  rest(target: number) {
    this.hold();
    const element = this.element;
    if (element.readyState >= 1 && !element.seeking && Math.abs(element.currentTime - target) > PAUSED_DRIFT) {
      this.seek(target);
    }
  }

  private seek(to: number) {
    this.seekStarted = performance.now();
    this.element.currentTime = to;
  }

  private rate(value: number) {
    this.catchingUp = value !== 1;
    if (this.element.playbackRate !== value) this.element.playbackRate = value;
  }
}

export class ProjectPlayer extends EventTarget implements MediaClock {
  private segments: PlayerSegment[] = [];
  private elements: [HTMLVideoElement, HTMLVideoElement];
  private followers: [VideoFollower, VideoFollower];
  private active = 0;
  private index = -1;
  private time = 0;
  private playing = false;
  /** Playing, but waiting for the sound and the picture to load before the clock starts. */
  private waiting = false;
  /** Counts each start of playing, so one waiting to start knows it was overtaken. */
  private run = 0;
  private pendingSeeks = 0;
  /** Where each element seeking goes once it lands, and whether that is a seek of the player's. */
  private queued = new Map<HTMLVideoElement, { time: number; report: boolean }>();
  /** The seeks counted in `pendingSeeks` of each element, to end with its file. */
  private tracked = new Map<HTMLVideoElement, Set<() => void>>();
  /** The seek each element makes once its file's metadata is in: only the last asked. */
  private onMetadata = new Map<HTMLVideoElement, () => void>();
  private frame = 0;
  private lastTimeUpdate = 0;
  private disposed = false;
  private readonly forward = (event: Event) => {
    if (event.target === this.elements[this.active]) {
      this.dispatchEvent(new Event(event.type));
    }
  };
  // Animation frames stop in hidden windows; media events still arrive there.
  private readonly fallback = (event: Event) => {
    if (this.playing && event.target === this.elements[this.active]) this.step();
  };

  constructor(
    first: HTMLVideoElement,
    second: HTMLVideoElement,
    private readonly clock: PlaybackClock,
  ) {
    super();
    this.elements = [first, second];
    this.followers = [new VideoFollower(first), new VideoFollower(second)];
    for (const element of this.elements) {
      element.preload = "auto";
      element.playsInline = true;
      // The clock plays the sound.
      element.muted = true;
      element.addEventListener("loadedmetadata", this.forward);
      element.addEventListener("timeupdate", this.fallback);
      element.addEventListener("ended", this.fallback);
    }
  }

  get currentTime() {
    return this.time;
  }

  get paused() {
    return !this.playing;
  }

  get seeking() {
    return this.pendingSeeks > 0 || this.queuedReport();
  }

  get readyState() {
    return this.index < 0 ? 0 : this.elements[this.active].readyState;
  }

  get playbackRate() {
    return 1;
  }

  get duration() {
    const last = this.segments[this.segments.length - 1];
    return last ? last.offset + last.end - last.start : 0;
  }

  /** The element showing the current clip. */
  get activeElement() {
    return this.elements[this.active];
  }

  get activeClipId() {
    return this.segments[this.index]?.clipId ?? null;
  }

  /** Replace the timeline, keeping the current position where possible. */
  setSegments(segments: PlayerSegment[]) {
    this.segments = segments;
    this.seek(Math.min(this.time, this.duration));
  }

  async play() {
    if (this.disposed || this.segments.length === 0 || this.playing) return;
    this.clock.resume?.();
    if (this.time >= this.duration - SWITCH_EARLY) this.seek(0);
    this.playing = true;
    this.dispatchEvent(new Event("play"));
    await this.begin();
  }

  pause() {
    if (!this.playing) return;
    this.run += 1;
    if (!this.waiting) this.time = Math.min(this.clock.time, this.duration);
    this.clock.stop();
    this.playing = false;
    this.waiting = false;
    cancelAnimationFrame(this.frame);
    for (const follower of this.followers) follower.hold();
    // Showing the frame where it stopped, which a video catching up may not.
    const segment = this.segments[this.index];
    if (segment) this.load(this.elements[this.active], segment, segment.start + this.time - segment.offset, true);
    this.dispatchEvent(new Event("pause"));
    this.emitTime(true);
  }

  toggle() {
    if (this.playing) this.pause();
    else void this.play();
  }

  seek(time: number) {
    if (this.disposed) return;
    const target = Math.max(0, Math.min(time, this.duration));
    this.time = target;
    const index = this.segmentAt(target);
    this.index = index;
    if (index < 0) {
      this.queued.clear();
      let ended = false;
      for (const element of this.elements) {
        ended = this.endSeeks(element) || ended;
        this.whenLoaded(element, null);
        element.pause();
        // Loaded again when it comes back.
        delete element.dataset.src;
        element.removeAttribute("src");
        element.load();
      }
      if (ended && this.pendingSeeks === 0) this.dispatchEvent(new Event("seeked"));
      this.emitActive();
      this.emitTime(true);
      return;
    }
    const segment = this.segments[index];
    const element = this.elements[this.active];
    this.followers[this.active].hold();
    this.load(element, segment, segment.start + target - segment.offset, true);
    this.followers[1 - this.active].hold();
    this.preloadNext();
    this.emitActive();
    this.emitTime(true);
    // Started again from here once it is loaded.
    if (this.playing) void this.begin();
    else this.clock.preload?.(target);
  }

  dispose() {
    this.disposed = true;
    this.queued.clear();
    cancelAnimationFrame(this.frame);
    this.clock.stop();
    for (const element of this.elements) {
      this.endSeeks(element);
      this.whenLoaded(element, null);
      element.removeEventListener("loadedmetadata", this.forward);
      element.removeEventListener("timeupdate", this.fallback);
      element.removeEventListener("ended", this.fallback);
      element.pause();
    }
    for (const follower of this.followers) follower.dispose();
  }

  /**
   * Start the clock at the current time once the sound and the picture there
   * are loaded, unless pausing or seeking overtook it meanwhile.
   */
  private async begin() {
    const run = ++this.run;
    this.clock.stop();
    cancelAnimationFrame(this.frame);
    if (!this.waiting) {
      this.waiting = true;
      this.dispatchEvent(new Event("waiting"));
    }
    const time = this.time;
    await Promise.all([this.clock.prepare(time), this.ready(run)]);
    if (run !== this.run || !this.playing || this.disposed) return;
    this.waiting = false;
    this.clock.start(time);
    this.dispatchEvent(new Event("playing"));
    this.emitTime(true);
    this.loop();
  }

  /**
   * Settles once the element showing the current clip can play on, with no
   * seek of the player's going on, or the wait is too long or overtaken.
   */
  private ready(run: number): Promise<void> {
    const started = performance.now();
    return new Promise((resolve) => {
      const check = () => {
        const element = this.elements[this.active];
        const loaded = element.readyState >= 3 && !element.seeking && !this.seeking;
        if (loaded || run !== this.run || this.disposed || performance.now() - started > READY_TIMEOUT_MS) {
          resolve();
        } else {
          setTimeout(check, READY_POLL_MS);
        }
      };
      check();
    });
  }

  private segmentAt(time: number) {
    for (let index = 0; index < this.segments.length; index++) {
      const segment = this.segments[index];
      if (time < segment.offset + segment.end - segment.start) return index;
    }
    return this.segments.length - 1;
  }

  private load(element: HTMLVideoElement, segment: PlayerSegment, sourceTime: number, report: boolean) {
    const seek = () => this.seekElement(element, sourceTime, report);
    if (element.dataset.src !== segment.url) {
      // A seek queued in the file before is of no use in this one, and one
      // going on ends without its "seeked".
      this.queued.delete(element);
      const ended = this.endSeeks(element);
      element.dataset.src = segment.url;
      element.src = segment.url;
      if (report) {
        this.trackSeek(element, "loadeddata", ended);
      } else if (ended && this.pendingSeeks === 0 && !this.queuedReport()) {
        this.dispatchEvent(new Event("seeked"));
        this.emitTime(true);
      }
      this.whenLoaded(element, seek);
    } else if (element.readyState >= 1) {
      this.whenLoaded(element, null);
      seek();
    } else {
      this.whenLoaded(element, seek);
    }
  }

  /** Make `seek` once `element` has its file's metadata, instead of any asked before; none for null. */
  private whenLoaded(element: HTMLVideoElement, seek: (() => void) | null) {
    const before = this.onMetadata.get(element);
    if (before) element.removeEventListener("loadedmetadata", before);
    this.onMetadata.delete(element);
    if (!seek) return;
    const run = () => {
      this.onMetadata.delete(element);
      seek();
    };
    this.onMetadata.set(element, run);
    element.addEventListener("loadedmetadata", run, { once: true });
  }

  /** Whether a seek of the player's waits on an element still seeking. The standby's preloading is not one. */
  private queuedReport(element?: HTMLVideoElement) {
    if (element) return this.queued.get(element)?.report ?? false;
    return [...this.queued.values()].some((queued) => queued.report);
  }

  /**
   * Seek `element` to `sourceTime`, or once it lands if it is seeking. A seek
   * `resumed` from the queue goes on with the one before, not a new one.
   * Returns whether it seeks now.
   */
  private seekElement(element: HTMLVideoElement, sourceTime: number, report: boolean, resumed = false): boolean {
    if (element.seeking) {
      if (!this.queued.has(element)) {
        // Also on an error, so that it never stays seeking.
        const resume = () => {
          element.removeEventListener("seeked", resume);
          element.removeEventListener("error", resume);
          this.resumeSeek(element);
        };
        element.addEventListener("seeked", resume);
        element.addEventListener("error", resume);
      }
      const before = this.queued.get(element);
      this.queued.set(element, { time: sourceTime, report: report || (before?.report ?? false) });
      return false;
    }
    if (Math.abs(element.currentTime - sourceTime) < 0.01) return false;
    if (report) this.trackSeek(element, "seeked", resumed);
    element.currentTime = sourceTime;
    return true;
  }

  /** Go on to where `element` was asked to go while it was seeking. */
  private resumeSeek(element: HTMLVideoElement) {
    if (this.disposed) return;
    const queued = this.queued.get(element);
    if (!queued) return;
    this.queued.delete(element);
    // Queued again if it is still seeking, as after an error.
    const seeking = this.seekElement(element, queued.time, queued.report, true);
    if (!seeking && queued.report && this.pendingSeeks === 0 && !this.queuedReport()) {
      this.dispatchEvent(new Event("seeked"));
      this.emitTime(true);
    }
  }

  /**
   * End the seeks counted on `element`, without their "seeked": its file is
   * replaced, which drops them. Returns whether there were any.
   */
  private endSeeks(element: HTMLVideoElement): boolean {
    const seeks = this.tracked.get(element);
    if (!seeks?.size) return false;
    for (const end of [...seeks]) end();
    return true;
  }

  /** Count a seek on `element` as pending until it lands. */
  private trackSeek(element: HTMLVideoElement, done = "seeked", resumed = false) {
    this.pendingSeeks += 1;
    // One seek going on to the next is not a new one.
    if (this.pendingSeeks === 1 && !resumed && !this.queuedReport()) this.dispatchEvent(new Event("seeking"));
    const seeks = this.tracked.get(element) ?? new Set<() => void>();
    this.tracked.set(element, seeks);
    const stop = () => {
      clearTimeout(timer);
      element.removeEventListener(done, finish);
      element.removeEventListener("error", finish);
      seeks.delete(stop);
      this.pendingSeeks = Math.max(0, this.pendingSeeks - 1);
    };
    seeks.add(stop);
    const finish = () => {
      stop();
      if (this.pendingSeeks === 0 && !this.queuedReport(element)) {
        this.dispatchEvent(new Event("seeked"));
        this.emitTime(true);
      }
    };
    // Never stay "seeking" forever when an event goes missing, nor wait on
    // the seek queued behind this one.
    const timer = setTimeout(() => {
      this.queued.delete(element);
      finish();
    }, 3000);
    element.addEventListener(done, finish);
    element.addEventListener("error", finish);
  }

  private preloadNext() {
    const next = this.segments[this.index + 1];
    if (next) this.load(this.elements[1 - this.active], next, next.start, false);
  }

  private loop = () => {
    if (!this.playing || this.disposed) return;
    this.step();
    this.frame = requestAnimationFrame(this.loop);
  };

  /** Follow the clock: show the clip at its time, switching at a cut. */
  private step() {
    if (!this.playing || this.waiting || this.index < 0) return;
    const time = this.clock.time;
    if (time >= this.duration) {
      this.pause();
      this.time = this.duration;
      this.dispatchEvent(new Event("ended"));
      return;
    }
    this.time = time;
    const index = this.segmentAt(time);
    if (index !== this.index) this.switchTo(index);
    const segment = this.segments[this.index];
    this.followers[this.active].play(segment.start + time - segment.offset, segment.end);
    this.emitTime(false);
  }

  /** Show clip `index`, reached while playing, on the standby element, which normally has it loaded. */
  private switchTo(index: number) {
    const next = this.segments[index];
    const previous = this.active;
    this.active = 1 - this.active;
    this.index = index;
    const element = this.elements[this.active];
    const sourceTime = next.start + this.time - next.offset;
    if (element.dataset.src !== next.url || Math.abs(element.currentTime - sourceTime) > 0.25) {
      this.load(element, next, sourceTime, false);
    }
    this.followers[previous].hold();
    this.preloadNext();
    this.emitActive();
    this.emitTime(true);
  }

  private emitActive() {
    this.dispatchEvent(new Event("activechange"));
  }

  private emitTime(force: boolean) {
    const now = performance.now();
    if (!force && now - this.lastTimeUpdate < TIME_UPDATE_INTERVAL_MS) return;
    this.lastTimeUpdate = now;
    this.dispatchEvent(new Event("timeupdate"));
  }
}
