// The project preview's sound, and the clock the preview plays by.
//
// Every sound of a project plays here through Web Audio: clips' own sound,
// that of video overlays and of the audio tracks. Each is a `Voice`, heard
// over a span of the project from a file, which the backend cuts into chunks
// of `CHUNK_SECONDS` on a fixed grid (src-tauri/src/preview_audio.rs). While
// playing, the chunks about to be heard are scheduled back to back on the
// audio context's own time, which runs at the sound card's pace, so the sound
// never waits on anything: the time it is at is the preview's clock, which
// the videos follow.
//
// What is heard at a time is what entered the context a little before, by
// the output's latency, so the clock tells the time of what is heard, as a
// picture shown with it should be of.

export const CHUNK_SECONDS = 10;
export const SAMPLE_RATE = 48000;
/** How far ahead of what enters the context chunks are scheduled. */
const LOOKAHEAD = 2.5;
/** How far ahead of what is heard chunks are loaded. */
const PREFETCH = 8;
/** What is loaded before playing starts. */
const PREPARE = 1;
const PREPARE_TIMEOUT_MS = 4000;
/** The first sound is scheduled this long after starting, so that it is not late. */
const START_LEAD = 0.05;
/** A chunk scheduled late starts this long after now, cut short at its head. */
const LATE_LEAD = 0.005;
const TICK_MS = 50;
/** Chunks kept decoded, of about 3.8 MB each. */
const MAX_CHUNKS = 24;
/** A chunk that failed to load is asked for again after this long. */
const RETRY_MS = 5000;
/** How quickly a change of volume is followed, in seconds. */
const VOLUME_SMOOTHING = 0.015;

/** A sound heard over a span of the project, from a file. */
export interface Voice {
  /** Stays the same while the same item is heard: changing its volume then does not start it again. */
  id: string;
  /** Relative to the output directory. */
  file: string;
  /** Project time it is heard over. */
  start: number;
  end: number;
  /** File time heard at `start`. */
  from: number;
  /** 0 to 1. */
  volume: number;
  /** Seconds it fades in over from `start` and out over until `end`, together no longer than it lasts. */
  fadeIn?: number;
  fadeOut?: number;
}

/** The parts of an AudioContext used here. */
export interface PreviewAudioContext {
  readonly currentTime: number;
  readonly destination: AudioNode;
  readonly outputLatency?: number;
  readonly baseLatency?: number;
  getOutputTimestamp?(): AudioTimestamp;
  createGain(): GainNode;
  createBufferSource(): AudioBufferSourceNode;
  decodeAudioData(data: ArrayBuffer): Promise<AudioBuffer>;
  resume(): Promise<void>;
  close?(): Promise<void>;
}

/** Fetch chunk `index` of `file`; null when it has no sound there. */
export type ChunkLoader = (file: string, index: number) => Promise<ArrayBuffer | null>;

interface Chunk {
  buffer: AudioBuffer | null;
  loading: Promise<void> | null;
  failedAt: number;
  usedAt: number;
}

/** A voice being heard since the clock started. */
interface Sounding {
  voice: Voice;
  volume: GainNode;
  fade: GainNode;
  /** Chunks scheduled, or found silent. */
  scheduled: Set<number>;
  nodes: Set<AudioBufferSourceNode>;
}

/** How loud a voice's fades leave it at project time `time`, 0 to 1. */
export function fadeAt(voice: Voice, time: number): number {
  let gain = 1;
  if (voice.fadeIn && voice.fadeIn > 0) gain = Math.min(gain, (time - voice.start) / voice.fadeIn);
  if (voice.fadeOut && voice.fadeOut > 0) gain = Math.min(gain, (voice.end - time) / voice.fadeOut);
  return Math.min(1, Math.max(0, gain));
}

/** The chunks of `voice`'s file heard over project time `from..to`. */
export function chunksOf(voice: Voice, from: number, to: number): number[] {
  const start = Math.max(from, voice.start);
  const end = Math.min(to, voice.end);
  if (end <= start) return [];
  const first = Math.floor(Math.max(0, voice.from + start - voice.start) / CHUNK_SECONDS);
  const last = Math.floor(Math.max(0, voice.from + end - voice.start - 1e-9) / CHUNK_SECONDS);
  return Array.from({ length: last - first + 1 }, (_, index) => first + index);
}

/** What makes a voice another sound, apart from its volume. */
function shape(voice: Voice) {
  return [voice.file, voice.start, voice.end, voice.from, voice.fadeIn ?? 0, voice.fadeOut ?? 0, voice.volume > 0].join("|");
}

const key = (file: string, index: number) => `${index}:${file}`;

export class PreviewAudio {
  private voices: Voice[] = [];
  private chunks = new Map<string, Chunk>();
  private sounding = new Map<string, Sounding>();
  private master: GainNode;
  private running = false;
  /** Project time entering the context at `contextStart`. */
  private origin = 0;
  private contextStart = 0;
  /** The clock as last told, which never goes back. */
  private told = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private volumeValue = 1;
  private mutedValue = false;

  constructor(
    private readonly context: PreviewAudioContext,
    private readonly load: ChunkLoader,
  ) {
    this.master = context.createGain();
    this.master.connect(context.destination);
  }

  get time(): number {
    if (!this.running) return this.told;
    const heard = this.origin + Math.max(0, this.heardContextTime() - this.contextStart);
    this.told = Math.max(this.told, heard);
    return this.told;
  }

  set volume(value: number) {
    this.volumeValue = value;
    this.applyVolume();
  }

  set muted(value: boolean) {
    this.mutedValue = value;
    this.applyVolume();
  }

  resume() {
    this.context.resume().catch(() => {});
  }

  /** Replace what is heard; a voice changed only in volume plays on, louder or quieter. */
  setVoices(voices: Voice[]) {
    this.voices = voices;
    const byId = new Map(voices.map((voice) => [voice.id, voice]));
    for (const [id, sounding] of this.sounding) {
      const next = byId.get(id);
      if (!next || shape(next) !== shape(sounding.voice)) {
        this.silence(sounding);
        this.sounding.delete(id);
      } else if (next.volume !== sounding.voice.volume) {
        sounding.volume.gain.setTargetAtTime(next.volume, this.context.currentTime, VOLUME_SMOOTHING);
        sounding.voice = next;
      }
    }
    if (this.running) this.tick();
  }

  /** Load what is heard from project time `time` on, waiting no longer than is reasonable. */
  async prepare(time: number): Promise<void> {
    const loads = this.audible().flatMap((voice) =>
      chunksOf(voice, time, time + PREPARE).map((index) => this.ensure(voice.file, index)),
    );
    if (loads.length === 0) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<void>((resolve) => (timer = setTimeout(resolve, PREPARE_TIMEOUT_MS)));
    await Promise.race([Promise.all(loads), timeout]);
    clearTimeout(timer);
  }

  /** Load, without playing, what is heard from `time`, as after a seek, so that playing from there starts at once. */
  preload(time: number) {
    for (const voice of this.audible()) {
      for (const index of chunksOf(voice, time, time + PREPARE)) void this.ensure(voice.file, index);
    }
  }

  /** Play from project time `time`. */
  start(time: number) {
    this.stop();
    this.running = true;
    this.origin = time;
    this.told = time;
    this.contextStart = this.context.currentTime + START_LEAD;
    this.tick();
    this.timer = setInterval(() => this.tick(), TICK_MS);
  }

  stop() {
    if (!this.running) return;
    this.told = this.time;
    this.running = false;
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
    for (const sounding of this.sounding.values()) this.silence(sounding);
    this.sounding.clear();
  }

  dispose() {
    this.stop();
    this.chunks.clear();
    this.master.disconnect();
    this.context.close?.().catch(() => {});
  }

  private audible() {
    return this.voices.filter((voice) => voice.volume > 0 && voice.end > voice.start);
  }

  /** The context's time of what is heard now. */
  private heardContextTime() {
    const now = this.context.currentTime;
    const stamp = this.context.getOutputTimestamp?.();
    if (stamp?.contextTime && stamp.performanceTime) {
      return Math.min(now, stamp.contextTime + (performance.now() - stamp.performanceTime) / 1000);
    }
    return now - (this.context.outputLatency || this.context.baseLatency || 0);
  }

  private contextTimeOf(projectTime: number) {
    return this.contextStart + projectTime - this.origin;
  }

  private projectTimeOf(contextTime: number) {
    return this.origin + contextTime - this.contextStart;
  }

  private applyVolume() {
    const value = this.mutedValue ? 0 : Math.min(1, Math.max(0, this.volumeValue));
    this.master.gain.setTargetAtTime(value, this.context.currentTime, VOLUME_SMOOTHING);
  }

  /** Schedule what enters the context soon and load what is heard after. */
  private tick() {
    if (!this.running) return;
    const now = this.context.currentTime;
    const entering = this.projectTimeOf(now);
    const heard = this.time;
    for (const voice of this.audible()) {
      if (voice.end <= entering) continue;
      for (const index of chunksOf(voice, heard, heard + PREFETCH)) void this.ensure(voice.file, index);
      const due = chunksOf(voice, entering, entering + LOOKAHEAD);
      if (due.length === 0) continue;
      let sounding = this.sounding.get(voice.id);
      if (!sounding) {
        sounding = this.sound(voice, now);
        this.sounding.set(voice.id, sounding);
      }
      for (const index of due) this.schedule(sounding, index, now);
    }
    this.evict();
  }

  /** Begin hearing `voice`: its volume, and its fades from now on. */
  private sound(voice: Voice, now: number): Sounding {
    const volume = this.context.createGain();
    const fade = this.context.createGain();
    volume.gain.value = voice.volume;
    fade.connect(volume);
    volume.connect(this.master);
    // The fades are straight lines between these times, as export fades.
    const from = Math.max(voice.start, this.projectTimeOf(now));
    fade.gain.setValueAtTime(fadeAt(voice, from), Math.max(now, this.contextTimeOf(from)));
    const edges = [voice.start + (voice.fadeIn ?? 0), voice.end - (voice.fadeOut ?? 0), voice.end];
    let last = from;
    for (const edge of edges) {
      if (edge <= last) continue;
      fade.gain.linearRampToValueAtTime(fadeAt(voice, edge), this.contextTimeOf(edge));
      last = edge;
    }
    return { voice, volume, fade, scheduled: new Set(), nodes: new Set() };
  }

  /** Schedule the part of chunk `index` that `sounding` plays, once it is loaded. */
  private schedule(sounding: Sounding, index: number, now: number) {
    if (sounding.scheduled.has(index)) return;
    const { voice } = sounding;
    const chunk = this.chunks.get(key(voice.file, index));
    if (!chunk || chunk.loading) {
      void this.ensure(voice.file, index);
      return;
    }
    // Failed: silent this time through.
    sounding.scheduled.add(index);
    if (!chunk.buffer) return;
    chunk.usedAt = performance.now();
    const chunkStart = index * CHUNK_SECONDS;
    const fileStart = Math.max(voice.from, chunkStart);
    const fileEnd = Math.min(voice.from + voice.end - voice.start, chunkStart + chunk.buffer.duration);
    let when = this.contextTimeOf(voice.start + fileStart - voice.from);
    let offset = fileStart - chunkStart;
    let length = fileEnd - fileStart;
    // Loaded too late to start on time: heard from now, or from where playing started.
    const earliest = Math.max(now + LATE_LEAD, this.contextStart);
    if (when < earliest) {
      offset += earliest - when;
      length -= earliest - when;
      when = earliest;
    }
    if (length <= 0.001) return;
    const node = this.context.createBufferSource();
    node.buffer = chunk.buffer;
    node.connect(sounding.fade);
    node.onended = () => {
      sounding.nodes.delete(node);
      node.disconnect();
    };
    node.start(when, offset, length);
    sounding.nodes.add(node);
  }

  private silence(sounding: Sounding) {
    for (const node of sounding.nodes) {
      node.onended = null;
      try {
        node.stop();
      } catch {
        // Never started.
      }
      node.disconnect();
    }
    sounding.nodes.clear();
    sounding.fade.disconnect();
    sounding.volume.disconnect();
  }

  /** Load chunk `index` of `file` unless it is, or is being loaded. */
  private ensure(file: string, index: number): Promise<void> {
    const existing = this.chunks.get(key(file, index));
    const now = performance.now();
    if (existing) {
      existing.usedAt = now;
      if (existing.loading) return existing.loading;
      if (existing.buffer || now - existing.failedAt < RETRY_MS) return Promise.resolve();
    }
    const chunk: Chunk = existing ?? { buffer: null, loading: null, failedAt: 0, usedAt: now };
    this.chunks.set(key(file, index), chunk);
    chunk.loading = this.load(file, index)
      .then((data) => (data ? this.context.decodeAudioData(data) : null))
      .then((buffer) => {
        chunk.buffer = buffer;
        if (!buffer) chunk.failedAt = performance.now();
      })
      .catch((error) => {
        console.warn(`Failed to load the preview's sound of ${file} at chunk ${index}:`, error);
        chunk.failedAt = performance.now();
      })
      .finally(() => {
        chunk.loading = null;
      });
    return chunk.loading;
  }

  /** Forget the chunks used longest ago beyond `MAX_CHUNKS`. */
  private evict() {
    if (this.chunks.size <= MAX_CHUNKS) return;
    const idle = [...this.chunks].filter(([, chunk]) => !chunk.loading).sort(([, a], [, b]) => a.usedAt - b.usedAt);
    for (const [name] of idle.slice(0, this.chunks.size - MAX_CHUNKS)) this.chunks.delete(name);
  }
}
