// Seamless preview of a project timeline with two <video> elements: while one
// plays a clip, the other is already loaded at the next clip's in point, and
// they swap at the cut. The player itself behaves like a media element whose
// time is project time, so components that follow a <video> (the libass
// subtitle preview) can follow the whole project instead.

export interface PlayerSegment {
  clipId: string;
  url: string;
  /** In and out points within the material. */
  start: number;
  end: number;
  /** Project time where the segment starts. */
  offset: number;
}

/** The parts of HTMLMediaElement that followers of a media clock read. */
export interface MediaClock extends EventTarget {
  readonly currentTime: number;
  readonly paused: boolean;
  readonly seeking: boolean;
  readonly readyState: number;
  readonly playbackRate: number;
}

// Switch slightly before the out point: browsers report time in steps, and a
// frame of the next clip is better than a frame past the out point.
const SWITCH_EARLY = 1 / 60;
const TIME_UPDATE_INTERVAL_MS = 100;

export class ProjectPlayer extends EventTarget implements MediaClock {
  private segments: PlayerSegment[] = [];
  private elements: [HTMLVideoElement, HTMLVideoElement];
  private active = 0;
  private index = -1;
  private time = 0;
  private playing = false;
  private pendingSeeks = 0;
  private frame = 0;
  private lastTimeUpdate = 0;
  private volumeValue = 1;
  private mutedValue = false;
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

  constructor(first: HTMLVideoElement, second: HTMLVideoElement) {
    super();
    this.elements = [first, second];
    for (const element of this.elements) {
      element.preload = "auto";
      element.playsInline = true;
      for (const type of ["waiting", "playing", "loadedmetadata"]) {
        element.addEventListener(type, this.forward);
      }
      element.addEventListener("timeupdate", this.fallback);
      element.addEventListener("ended", this.fallback);
    }
    this.applyVolume();
  }

  get currentTime() {
    return this.time;
  }

  get paused() {
    return !this.playing;
  }

  get seeking() {
    return this.pendingSeeks > 0;
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

  set volume(value: number) {
    this.volumeValue = value;
    this.applyVolume();
  }

  set muted(value: boolean) {
    this.mutedValue = value;
    this.applyVolume();
  }

  /** Replace the timeline, keeping the current position where possible. */
  setSegments(segments: PlayerSegment[]) {
    this.segments = segments;
    this.seek(Math.min(this.time, this.duration));
  }

  async play() {
    if (this.disposed || this.segments.length === 0) return;
    if (this.time >= this.duration - SWITCH_EARLY) this.seek(0);
    this.playing = true;
    this.dispatchEvent(new Event("play"));
    await this.playActive();
    this.loop();
  }

  pause() {
    if (!this.playing) return;
    this.playing = false;
    cancelAnimationFrame(this.frame);
    for (const element of this.elements) element.pause();
    this.syncTime();
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
      for (const element of this.elements) {
        element.pause();
        element.removeAttribute("src");
        element.load();
      }
      this.emitActive();
      this.emitTime(true);
      return;
    }
    const segment = this.segments[index];
    const element = this.elements[this.active];
    if (!this.playing) element.pause();
    this.load(element, segment, segment.start + target - segment.offset, true);
    this.elements[1 - this.active].pause();
    if (this.playing) void this.playActive();
    this.preloadNext();
    this.emitActive();
    this.emitTime(true);
  }

  dispose() {
    this.disposed = true;
    cancelAnimationFrame(this.frame);
    for (const element of this.elements) {
      for (const type of ["waiting", "playing", "loadedmetadata"]) {
        element.removeEventListener(type, this.forward);
      }
      element.removeEventListener("timeupdate", this.fallback);
      element.removeEventListener("ended", this.fallback);
      element.pause();
    }
  }

  private segmentAt(time: number) {
    for (let index = 0; index < this.segments.length; index++) {
      const segment = this.segments[index];
      if (time < segment.offset + segment.end - segment.start) return index;
    }
    return this.segments.length - 1;
  }

  private applyVolume() {
    this.elements.forEach((element, index) => {
      element.volume = this.volumeValue;
      // The standby element must never be heard while it preloads.
      element.muted = this.mutedValue || index !== this.active;
    });
  }

  private load(element: HTMLVideoElement, segment: PlayerSegment, sourceTime: number, report: boolean) {
    const seek = () => {
      if (Math.abs(element.currentTime - sourceTime) < 0.01) return;
      if (report) this.trackSeek(element);
      element.currentTime = sourceTime;
    };
    if (element.dataset.src !== segment.url) {
      element.dataset.src = segment.url;
      element.src = segment.url;
      if (report) this.trackSeek(element, "loadeddata");
      element.addEventListener("loadedmetadata", seek, { once: true });
    } else if (element.readyState >= 1) {
      seek();
    } else {
      element.addEventListener("loadedmetadata", seek, { once: true });
    }
  }

  /** Count a seek on `element` as pending until it lands. */
  private trackSeek(element: HTMLVideoElement, done = "seeked") {
    this.pendingSeeks += 1;
    if (this.pendingSeeks === 1) this.dispatchEvent(new Event("seeking"));
    const finish = () => {
      clearTimeout(timer);
      element.removeEventListener(done, finish);
      element.removeEventListener("error", finish);
      this.pendingSeeks = Math.max(0, this.pendingSeeks - 1);
      if (this.pendingSeeks === 0) {
        this.dispatchEvent(new Event("seeked"));
        this.emitTime(true);
      }
    };
    // Never stay "seeking" forever when an event goes missing.
    const timer = setTimeout(finish, 3000);
    element.addEventListener(done, finish);
    element.addEventListener("error", finish);
  }

  private preloadNext() {
    const next = this.segments[this.index + 1];
    if (next) this.load(this.elements[1 - this.active], next, next.start, false);
  }

  private async playActive() {
    try {
      await this.elements[this.active].play();
    } catch (error) {
      // Interrupted by a seek or switch, which restarts playback itself.
      if ((error as DOMException)?.name !== "AbortError") {
        console.warn("Preview playback failed:", error);
        this.pause();
      }
    }
  }

  private syncTime() {
    const segment = this.segments[this.index];
    if (!segment || this.seeking) return;
    const element = this.elements[this.active];
    const sourceTime = Math.min(Math.max(element.currentTime, segment.start), segment.end);
    this.time = segment.offset + sourceTime - segment.start;
  }

  private loop = () => {
    if (!this.playing || this.disposed) return;
    this.step();
    this.frame = requestAnimationFrame(this.loop);
  };

  /** Follow the active element, switching at the out point. */
  private step() {
    const segment = this.segments[this.index];
    const element = this.elements[this.active];
    if (!segment || this.seeking) return;
    if (element.currentTime >= segment.end - SWITCH_EARLY || element.ended) {
      this.advance();
    } else {
      this.syncTime();
      this.emitTime(false);
    }
  }

  private advance() {
    const nextIndex = this.index + 1;
    const next = this.segments[nextIndex];
    if (!next) {
      this.time = this.duration;
      this.pause();
      this.dispatchEvent(new Event("ended"));
      return;
    }
    const previous = this.elements[this.active];
    this.active = 1 - this.active;
    this.index = nextIndex;
    this.time = next.offset;
    const element = this.elements[this.active];
    // The standby element normally holds this clip already; load it otherwise.
    if (element.dataset.src !== next.url || Math.abs(element.currentTime - next.start) > 0.25) {
      this.load(element, next, next.start, true);
    }
    this.applyVolume();
    previous.pause();
    void this.playActive();
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
