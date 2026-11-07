// What players draw recorded danmaku with: the floating nodes, in the room's
// default look and the danmaku style setting, and a driver that hands
// recorded danmaku over as a video reaches them (see lib/danmaku-feed.ts),
// laid out as in Bilibili's live room (see lib/danmaku-layout.ts).

import { invoke } from "./invoker";
import type { Config, Danmu2AssOptions, DanmuEntry } from "./interface";
import { renderDanmuContent } from "./danmu-content";
import { formatScPrice, getScColors } from "./superchat";
import { createFloatingOverlay, observeResize } from "./floating-overlay";
import { createDanmakuFeed, FEED_TICK_MS } from "./danmaku-feed";
import {
  createDanmakuSpace,
  createPlacer,
  danmakuFontSize,
  LINE_HEIGHT,
  paceTimes,
} from "./danmaku-layout";

/**
 * The danmaku style setting, read again whenever the window comes back, as
 * it is changed in the main window. It applies to danmaku drawn after.
 */
export function watchDanmakuStyle() {
  let style: Danmu2AssOptions = { font_size: 36, opacity: 0.8 };
  async function load() {
    try {
      const config = await invoke<Config>("get_config");
      if (config.danmu_ass_options) style = config.danmu_ass_options;
    } catch (error) {
      console.warn("Failed to load the danmaku style:", error);
    }
  }
  void load();
  window.addEventListener("focus", load);
  return {
    get: () => style,
    stop: () => window.removeEventListener("focus", load),
  };
}

/**
 * A danmaku in the room's default look, bold and outlined, in the font, size
 * and opacity burned into videos, scaled to a player `height` high.
 */
export function createDanmakuNode(record: DanmuEntry, style: Danmu2AssOptions, height: number) {
  const danmaku = document.createElement("p");
  danmaku.style.color = "white";
  danmaku.style.fontSize = `${danmakuFontSize(style.font_size, height)}px`;
  danmaku.style.opacity = `${style.opacity}`;
  danmaku.style.fontFamily = "'Microsoft YaHei', 微软雅黑, sans-serif";
  danmaku.style.fontWeight = "bold";
  danmaku.style.lineHeight = `${LINE_HEIGHT}`;
  danmaku.style.whiteSpace = "nowrap";
  danmaku.style.pointerEvents = "none";
  danmaku.style.margin = "0";
  danmaku.style.padding = "0";
  danmaku.style.zIndex = "500";
  danmaku.style.textShadow = "1px 0 1px #000, 0 1px 1px #000, 0 -1px 1px #000, -1px 0 1px #000";
  const dispose = renderDanmuContent(danmaku, { ...record, content: record.content.trim() }, LINE_HEIGHT);
  return { node: danmaku, dispose };
}

/** A super chat as an SC card whose colors depend on the price (see lib/superchat.ts). */
export function createSuperChatCard(record: DanmuEntry) {
  const price = record.price ?? 0;
  const colors = getScColors(price);

  const card = document.createElement("div");
  card.style.position = "absolute";
  card.style.display = "flex";
  card.style.flexDirection = "column";
  card.style.maxWidth = "min(420px, 60%)";

  const head = document.createElement("div");
  head.style.display = "flex";
  head.style.alignItems = "center";
  head.style.justifyContent = "space-between";
  head.style.gap = "8px";
  head.style.padding = "3px 10px";
  head.style.backgroundColor = colors.info;
  head.style.border = `1px solid ${colors.content}`;
  head.style.borderRadius = "8px 8px 0 0";
  head.style.color = "#333";
  head.style.fontSize = "0.9em";
  const priceEl = document.createElement("span");
  priceEl.style.fontWeight = "700";
  priceEl.style.flexShrink = "0";
  priceEl.style.color = colors.content;
  priceEl.textContent = formatScPrice(price);
  const nameEl = document.createElement("span");
  nameEl.style.opacity = "0.78";
  nameEl.style.minWidth = "0";
  nameEl.style.overflow = "hidden";
  nameEl.style.textOverflow = "ellipsis";
  nameEl.style.whiteSpace = "nowrap";
  nameEl.textContent = record.user_name ?? "";
  head.appendChild(nameEl);
  head.appendChild(priceEl);

  const body = document.createElement("div");
  body.style.padding = "3px 10px";
  body.style.backgroundColor = colors.content;
  body.style.borderLeft = `1px solid ${colors.content}`;
  body.style.borderRight = `1px solid ${colors.content}`;
  body.style.borderBottom = `1px solid ${colors.content}`;
  body.style.borderRadius = "0 0 8px 8px";
  body.style.color = "white";
  // Wrap long messages within the card width so the full content stays
  // readable while the card floats across the screen.
  body.style.whiteSpace = "pre-wrap";
  body.style.wordBreak = "break-word";
  body.textContent = record.content;

  card.appendChild(head);
  card.appendChild(body);

  card.style.pointerEvents = "none";
  card.style.zIndex = "500";
  card.style.boxShadow = "0 2px 8px rgba(0, 0, 0, 0.35)";
  return card;
}

/**
 * When each recorded message is drawn (ms), or null when it is dropped:
 * danmaku go through the room's queue on the video's clock, as burned videos
 * do; super chats keep their own times.
 */
export function pacedDanmakuTimes(records: DanmuEntry[]): (number | null)[] {
  const queued = records.flatMap((entry, index) => (entry.type === "super_chat" ? [] : [index]));
  const drawn = paceTimes(queued.map((index) => records[index].ts));
  const times = records.map((entry) => (entry.type === "super_chat" ? entry.ts : null));
  queued.forEach((index, i) => (times[index] = drawn[i]));
  return times;
}

/**
 * Draw the recorded danmaku of `video`, timed in ms of it, into `overlay`,
 * which covers its picture: each as playback reaches it, moving with the
 * video, and after a seek or a resize, those in flight there.
 */
export function attachRecordedDanmaku(
  video: HTMLVideoElement,
  overlay: HTMLElement,
  records: DanmuEntry[],
  style: () => Danmu2AssOptions,
) {
  const floating = createFloatingOverlay(overlay);
  const space = createDanmakuSpace();
  const feed = createDanmakuFeed();
  const times = pacedDanmakuTimes(records);
  const clock = () => video.currentTime;
  const container = () => ({ width: overlay.clientWidth, height: overlay.clientHeight });
  let enabled = true;

  // On screen, they pause with the video, also while it waits for data, and
  // follow its speed.
  let waiting = false;
  const syncPlayback = () => floating.setPlayback(video.paused || waiting, video.playbackRate);
  const listeners: [string, () => void][] = [
    ["seeking", () => feed.reset()],
    ["waiting", () => ((waiting = true), syncPlayback())],
    ["playing", () => ((waiting = false), syncPlayback())],
    ["play", syncPlayback],
    ["pause", syncPlayback],
    ["ratechange", syncPlayback],
  ];
  for (const [type, listener] of listeners) video.addEventListener(type, listener);
  syncPlayback();

  function clear() {
    floating.clear();
    space.clear();
  }

  function draw(entry: DanmuEntry, at: number) {
    if (entry.type === "super_chat") {
      // Paid messages are never dropped: on a full screen they wait for room.
      const place = createPlacer(space, clock, container, { at, keep: true });
      floating.add(createSuperChatCard(entry), place, { keep: true, follow: true });
    } else {
      const { node, dispose } = createDanmakuNode(entry, style(), overlay.clientHeight);
      floating.add(node, createPlacer(space, clock, container, { at }), { dispose, follow: true });
    }
  }

  function step() {
    if (!enabled) {
      feed.reset();
      return;
    }
    const now = Math.floor(clock() * 1000);
    const next = feed.step({ now, offset: 0, paused: video.paused, atLiveEdge: false });
    if (next.clear) clear();
    if (next.from === null) return;
    const due: [number, DanmuEntry][] = [];
    records.forEach((entry, index) => {
      const time = times[index];
      if (time !== null && time >= (next.from as number) && time < now) due.push([time, entry]);
    });
    due.sort((a, b) => a[0] - b[0]);
    for (const [time, entry] of due) draw(entry, time / 1000);
  }
  const timer = setInterval(step, FEED_TICK_MS);
  step();
  // A new size starts over, like a seek, laying out what is in flight anew.
  const stopResize = observeResize(overlay, () => {
    feed.reset();
    step();
  });

  return {
    /** Show or hide them; shown again, those in flight come back. */
    setEnabled(value: boolean) {
      enabled = value;
      if (!value) clear();
      step();
    },
    destroy() {
      clearInterval(timer);
      stopResize();
      for (const [type, listener] of listeners) video.removeEventListener(type, listener);
      // The overlay is the caller's; only what was drawn into it goes.
      clear();
    },
  };
}
