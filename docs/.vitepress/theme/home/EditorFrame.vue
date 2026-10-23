<script setup lang="ts">
/** The clip project editor window, driven by the editor and subtitle scenes. */
import { computed } from "vue";
import Icon from "./Icon.vue";
import Thumb from "./Thumb.vue";

export type Span = { start: number; length: number; label: string };
export type Clip = Span & { scene: "dusk" | "city" | "studio" | "sea" | "forest" };

const props = defineProps<{
  clips: Clip[];
  texts: Span[];
  sounds: Span[];
  subtitles: Span[];
  time: number;
  playing: boolean;
  panel: "materials" | "text";
  danmu: boolean;
  /** The text overlay drawn on the preview, if any. */
  caption?: string;
  /** Name of the selected main-track clip, for the toolbar hint. */
  selected?: string;
  /** Materials in the bin that are already on the timeline. */
  used?: number[];
  recognizing?: boolean;
}>();

/** Seconds shown across the timeline. */
const VIEW = 150;
const at = (seconds: number) => `${(seconds / VIEW) * 100}%`;
const pad = (n: number) => String(Math.floor(n)).padStart(2, "0");
const timecode = (s: number) => `${pad(s / 60)}:${pad(s % 60)}.${pad((s % 1) * 100)}`;

const duration = computed(() =>
  props.clips.reduce((end, clip) => Math.max(end, clip.start + clip.length), 0),
);
const current = computed(
  () => props.clips.find((clip) => props.time >= clip.start && props.time < clip.start + clip.length) ?? props.clips[0],
);
const subtitle = computed(
  () => props.subtitles.find((item) => props.time >= item.start && props.time < item.start + item.length)?.label,
);

const bin = [
  { name: "深渊满星挑战 选区 1", note: "开场准备", length: "00:52", scene: "dusk" },
  { name: "深渊满星挑战 选区 2", note: "雷神开大", length: "00:47", scene: "forest" },
  { name: "深渊满星挑战 选区 3", note: "最后一波", length: "00:41", scene: "sea" },
  { name: "深夜杂谈 选区 1", note: "新番吐槽", length: "02:10", scene: "studio" },
  { name: "城市夜景 选区 4", note: "", length: "01:05", scene: "city" },
] as const;

const styles = ["默认", "描边", "综艺", "霓虹", "字幕条", "手写"];
const rail = [
  { id: "materials", label: "素材", icon: "film" },
  { id: "audio", label: "音频", icon: "music" },
  { id: "text", label: "文字", icon: "type" },
  { id: "sticker", label: "贴纸", icon: "smile" },
];
</script>

<template>
  <div class="editor">
    <header class="titlebar">
      <span class="title-input">深渊满星集锦</span>
      <span class="save-state">已保存</span>
      <span class="actions">
        <span class="icon-button"><Icon name="undo-2" :size="16" /></span>
        <span class="icon-button"><Icon name="redo-2" :size="16" /></span>
        <span class="icon-button"><Icon name="circle-help" :size="16" /></span>
        <span class="icon-button"><Icon name="list-checks" :size="16" /></span>
        <span class="divider" />
        <span class="toggle" data-ref="danmu" :class="{ on: danmu }">
          <Icon name="message-square-text" :size="14" />弹幕{{ danmu ? "：开" : "：关" }}
        </span>
        <span class="tb-button" data-ref="upload"><Icon name="upload" :size="14" />投稿</span>
        <span class="tb-button primary"><Icon name="download" :size="14" />导出</span>
      </span>
    </header>

    <div class="body">
      <nav class="rail">
        <span
          v-for="item in rail"
          :key="item.id"
          class="rail-button"
          :data-ref="`rail-${item.id}`"
          :class="{ active: panel === item.id }"
        >
          <Icon :name="item.icon" :size="18" />{{ item.label }}
        </span>
      </nav>

      <aside v-if="panel === 'materials'" class="bin">
        <div class="bin-head">
          <h2>素材库</h2>
          <div class="scope"><span class="active">本直播间</span><span>全部</span></div>
          <span class="search"><Icon name="search" :size="13" />搜索素材</span>
        </div>
        <div class="bin-list">
          <div
            v-for="(item, index) in bin"
            :key="item.name"
            class="item"
            :data-ref="`bin-${index}`"
            :class="{ used: used?.includes(index) }"
          >
            <span class="bin-thumb">
              <Thumb :scene="item.scene" />
              <span class="length num">{{ item.length }}</span>
            </span>
            <span class="meta">
              <span class="name">{{ item.name }}</span>
              <span class="note">{{ item.note || "22603245" }}</span>
            </span>
            <span class="add"><Icon name="plus" :size="14" /></span>
          </div>
        </div>
      </aside>

      <aside v-else class="bin text-panel">
        <div class="bin-head"><h2>文字</h2></div>
        <span class="add-text" data-ref="add-text"><Icon name="plus" :size="14" />添加文字</span>
        <div class="styles">
          <span
            v-for="(name, index) in styles"
            :key="name"
            class="style"
            :data-ref="`style-${index}`"
            :class="`s${index}`"
          >
            <b>文字</b>
            <small>{{ name }}</small>
          </span>
        </div>
      </aside>

      <div class="center">
        <div class="overlay-toolbar">
          <span v-if="selected" class="hint">主轨片段 · {{ selected }}</span>
          <span v-else-if="caption" class="hint">文字 · {{ caption }}</span>
          <span v-else class="hint">
            从左侧添加文字、贴纸，或把素材拖到时间轴主轨上方的视频轨道上叠加画面
          </span>
        </div>
        <div class="preview">
          <div class="frame">
            <Thumb v-if="current" :scene="current.scene" />
            <div v-else class="empty">
              <Icon name="film" :size="32" />从左侧素材库添加素材开始剪辑
            </div>
            <Transition name="m-pop">
              <span v-if="caption" class="caption">{{ caption }}</span>
            </Transition>
            <span v-if="subtitle" class="subtitle-line">{{ subtitle }}</span>
          </div>
        </div>
      </div>

      <aside class="inspector">
        <div class="inspector-heading">字幕{{ subtitles.length ? ` · ${subtitles.length}` : "" }}</div>
        <div class="inspector-content">
          <div class="panel-toolbar">
            <span class="tool-button"><Icon name="settings" :size="14" />样式</span>
            <span class="tool-button"><Icon name="eraser" :size="14" />清空</span>
            <span class="tool-button" data-ref="ai" :class="{ busy: recognizing }">
              <Icon :name="recognizing ? 'loader-circle' : 'brain-circuit'" :size="14" />
              {{ recognizing ? "识别中" : "AI 字幕" }}
            </span>
          </div>
          <TransitionGroup v-if="subtitles.length" name="m-rise" tag="div" class="rows">
            <div
              v-for="(item, index) in subtitles"
              :key="item.start"
              class="row"
              :class="{ playing: item.label === subtitle }"
            >
              <span class="row-index num">{{ index + 1 }}</span>
              <span class="row-text">{{ item.label }}</span>
            </div>
          </TransitionGroup>
          <div v-else class="empty-state">
            <strong>还没有字幕</strong>
            <span>点击「AI 字幕」识别，或在时间轴播放头处添加</span>
          </div>
        </div>
      </aside>
    </div>

    <div class="transport">
      <span class="group">
        <span class="icon-button"><Icon name="scissors" :size="16" />分割</span>
        <span class="icon-button"><Icon name="trash-2" :size="16" />删除</span>
        <span class="icon-button"><Icon name="plus" :size="16" />字幕</span>
      </span>
      <span class="group center-group">
        <span class="play-button" data-ref="play"><Icon :name="playing ? 'pause' : 'play'" :size="16" /></span>
        <span class="clock num">{{ timecode(time) }} / {{ timecode(duration) }}</span>
      </span>
      <span class="group end">
        <span class="zoom-label">缩放</span>
        <span class="range"><i /></span>
        <Icon name="volume-2" :size="16" />
        <span class="range short"><i /></span>
      </span>
    </div>

    <div class="timeline">
      <div class="labels">
        <div class="label ruler-label">时间</div>
        <div class="label sub"><Icon name="captions" :size="14" class="c-blue" />字幕</div>
        <div class="label"><Icon name="sticker" :size="14" class="c-amber" />贴图</div>
        <div class="label main">
          <Icon name="film" :size="14" class="c-violet" />
          <span class="stack">视频<small>主轨</small></span>
        </div>
        <div class="label"><Icon name="music" :size="14" class="c-green" />音频</div>
      </div>
      <div class="tracks" data-ref="tracks">
        <div class="ruler num">
          <span v-for="k in 6" :key="k" :style="{ left: at((k - 1) * 30) }">{{ pad(((k - 1) * 30) / 60) }}:{{ pad(((k - 1) * 30) % 60) }}</span>
        </div>
        <div class="track sub">
          <TransitionGroup name="m-fade">
            <span
              v-for="item in subtitles"
              :key="item.start"
              class="subtitle"
              :style="{ left: at(item.start), width: at(item.length) }"
              >{{ item.label }}</span
            >
          </TransitionGroup>
        </div>
        <div class="track">
          <TransitionGroup name="m-pop">
            <span
              v-for="item in texts"
              :key="item.start"
              class="overlay text"
              :style="{ left: at(item.start), width: at(item.length) }"
            >
              <Icon name="type" :size="12" />{{ item.label }}
            </span>
          </TransitionGroup>
        </div>
        <div class="track main" data-ref="main-track">
          <TransitionGroup name="m-pop">
            <span
              v-for="clip in clips"
              :key="clip.label"
              class="clip"
              :class="{ selected: clip.label === selected }"
              :style="{ left: at(clip.start), width: at(clip.length) }"
            >
              <span class="clip-head"><span class="clip-name">{{ clip.label }}</span><span class="num">00:{{ pad(clip.length) }}</span></span>
              <span class="wave" />
            </span>
          </TransitionGroup>
          <span class="drop-slot" :style="{ left: at(duration) }" data-ref="drop" />
        </div>
        <div class="track">
          <span
            v-for="item in sounds"
            :key="item.start"
            class="sound"
            :style="{ left: at(item.start), width: at(item.length) }"
          >
            <Icon name="music" :size="11" />{{ item.label }}
          </span>
        </div>
        <span class="playhead" :class="{ moving: playing }" :style="{ left: at(time) }" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor {
  display: flex;
  width: 100%;
  height: 100%;
  flex-direction: column;
  background: #1c1c1e;
  color: #fff;
}

.titlebar {
  display: flex;
  height: 40px;
  flex-shrink: 0;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid #222a35;
  background: #0e1219;
  padding: 0 12px 0 84px;
}

.title-input {
  border-radius: 7px;
  padding: 3px 8px;
  font-size: 13px;
  font-weight: 500;
  color: #e5e7eb;
}

.save-state {
  font-size: 11px;
  color: #6b7280;
}

.actions {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.icon-button {
  display: inline-flex;
  height: 28px;
  min-width: 28px;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border-radius: 7px;
  padding: 0 6px;
  font-size: 11px;
  color: #c4cfdd;
}

.divider {
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: #2b3340;
}

.toggle,
.tb-button {
  display: inline-flex;
  height: 28px;
  align-items: center;
  gap: 6px;
  border: 1px solid #344052;
  border-radius: 7px;
  background: #202733;
  padding: 0 11px;
  font-size: 12px;
  font-weight: 500;
  color: #dce5f3;
  transition: all 200ms ease;
}

.toggle {
  color: #8c98aa;
}

.toggle.on {
  border-color: rgb(10 132 255 / 70%);
  background: #153b5c;
  color: #65c2ff;
}

.tb-button.primary {
  border-color: transparent;
  background: #0a84ff;
  color: #fff;
}

.body {
  display: flex;
  min-height: 0;
  flex: 1;
}

.rail {
  display: flex;
  width: 56px;
  flex-shrink: 0;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  border-right: 1px solid #222a35;
  background: #11151c;
  padding: 10px 0;
}

.rail-button {
  display: flex;
  width: 44px;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  border-radius: 8px;
  padding: 7px 0;
  font-size: 11px;
  color: #8c98aa;
  transition: background-color 150ms ease, color 150ms ease;
}

.rail-button.active {
  background: rgb(10 132 255 / 18%);
  color: #55b9ff;
}

.bin {
  display: flex;
  width: 232px;
  flex-shrink: 0;
  flex-direction: column;
  border-right: 1px solid #222a35;
  background: #171c25;
}

.bin-head {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 12px 8px;
}

.bin-head h2 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: #e9eef8;
}

.scope {
  display: grid;
  grid-template-columns: 1fr 1fr;
  border-radius: 8px;
  background: #0d1118;
  padding: 2px;
  font-size: 11px;
  color: #8c98aa;
  text-align: center;
}

.scope span {
  border-radius: 6px;
  padding: 3px 0;
}

.scope .active {
  background: #293341;
  color: #e9eef8;
}

.search {
  display: flex;
  height: 28px;
  align-items: center;
  gap: 6px;
  border: 1px solid #293442;
  border-radius: 7px;
  background: #0d1118;
  padding: 0 8px;
  font-size: 11px;
  color: #6f7b8d;
}

.bin-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 0 6px;
}

.item {
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 8px;
  padding: 5px 6px;
  transition: background-color 200ms ease;
}

.item.used .name::after {
  margin-left: 4px;
  color: #8b5cf6;
  content: "●";
  font-size: 7px;
  vertical-align: 2px;
}

.bin-thumb {
  position: relative;
  width: 64px;
  height: 36px;
  flex-shrink: 0;
  overflow: hidden;
  border-radius: 5px;
}

.length {
  position: absolute;
  right: 2px;
  bottom: 2px;
  border-radius: 3px;
  background: rgb(0 0 0 / 60%);
  padding: 0 3px;
  font-size: 9px;
  color: #fff;
}

.meta {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
}

.name {
  overflow: hidden;
  font-size: 11px;
  color: #dce5f3;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.note {
  font-size: 10px;
  color: #6f7b8d;
}

.add {
  display: flex;
  width: 22px;
  height: 22px;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  color: #8c98aa;
}

.add-text {
  display: flex;
  height: 30px;
  align-items: center;
  justify-content: center;
  gap: 6px;
  margin: 0 12px;
  border-radius: 8px;
  background: #0a84ff;
  font-size: 12px;
  font-weight: 500;
}

.styles {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  padding: 12px;
}

.style {
  display: flex;
  height: 64px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border: 1px solid #293442;
  border-radius: 8px;
  background: #0d1118;
}

.style b {
  font-size: 18px;
}

.style small {
  font-size: 10px;
  color: #8c98aa;
}

.s1 b {
  color: #fff;
  -webkit-text-stroke: 3px #0a84ff;
  paint-order: stroke;
}

.s2 b {
  color: #ffd43b;
  -webkit-text-stroke: 3px #c92a2a;
  paint-order: stroke;
  font-style: italic;
}

.s3 b {
  color: #fff;
  text-shadow: 0 0 6px #f06595, 0 0 12px #f06595;
}

.s4 b {
  border-radius: 4px;
  background: rgb(0 0 0 / 70%);
  padding: 0 6px;
  font-size: 14px;
}

.s5 b {
  font-family: "Kaiti SC", "STKaiti", "KaiTi", serif;
  color: #ffe8cc;
}

.center {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
}

.overlay-toolbar {
  display: flex;
  height: 42px;
  flex-shrink: 0;
  align-items: center;
  border-bottom: 1px solid #222a35;
  background: #171c25;
  padding: 0 12px;
}

.hint {
  overflow: hidden;
  font-size: 11px;
  color: #8c98aa;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview {
  display: flex;
  min-height: 0;
  flex: 1;
  align-items: center;
  justify-content: center;
  background: #000;
  padding: 12px;
}

.frame {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  max-height: 100%;
  overflow: hidden;
  background: #000;
}

.empty {
  display: flex;
  height: 100%;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 12px;
  color: #6b7280;
}

.caption {
  position: absolute;
  top: 12%;
  left: 50%;
  translate: -50% 0;
  font-size: 30px;
  font-weight: 800;
  font-style: italic;
  color: #ffd43b;
  white-space: nowrap;
  -webkit-text-stroke: 6px #c92a2a;
  paint-order: stroke;
}

.caption::after {
  position: absolute;
  inset: -6px -10px;
  border: 1px dashed #c4b5fd;
  content: "";
}

.subtitle-line {
  position: absolute;
  bottom: 7%;
  left: 50%;
  translate: -50% 0;
  font-size: 17px;
  font-weight: 600;
  color: #fff;
  white-space: nowrap;
  text-shadow: 0 0 3px #000, 0 0 3px #000, 0 1px 4px #000;
}

.inspector {
  display: flex;
  width: 270px;
  flex-shrink: 0;
  flex-direction: column;
  border-left: 1px solid #222a35;
  background: #171c25;
}

.inspector-heading {
  padding: 14px 16px 10px;
  font-size: 13px;
  font-weight: 600;
  color: #e9eef8;
}

.inspector-content {
  position: relative;
  min-height: 0;
  flex: 1;
  overflow: hidden;
  padding: 0 16px 12px;
}

.panel-toolbar {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 6px;
  margin-bottom: 10px;
}

.tool-button {
  display: inline-flex;
  height: 30px;
  align-items: center;
  justify-content: center;
  gap: 5px;
  border: 1px solid #293442;
  border-radius: 8px;
  background: #202733;
  font-size: 11px;
  color: #c4cfdd;
  white-space: nowrap;
}

.tool-button.busy {
  color: #55b9ff;
}

.tool-button.busy :deep(svg) {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.rows {
  display: flex;
  flex-direction: column;
  border: 1px solid #293442;
  border-radius: 8px;
  overflow: hidden;
}

.row {
  display: flex;
  height: 32px;
  flex-shrink: 0;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid #222a35;
  padding: 0 10px;
  font-size: 12px;
  color: #dce5f3;
  transition: background-color 200ms ease;
}

.row:last-child {
  border-bottom: 0;
}

.row.playing {
  background: rgb(10 132 255 / 14%);
}

.row-index {
  width: 16px;
  font-size: 10px;
  color: #6f7b8d;
}

.row-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  margin-top: 60px;
  text-align: center;
}

.empty-state strong {
  font-size: 12px;
  font-weight: 500;
  color: #c4cfdd;
}

.empty-state span {
  font-size: 11px;
  color: #6f7b8d;
}

.transport {
  display: grid;
  height: 40px;
  flex-shrink: 0;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  border-top: 1px solid #222a35;
  background: #1c1c1e;
  padding: 0 10px;
}

.group {
  display: flex;
  align-items: center;
  gap: 4px;
  color: #c4cfdd;
}

.center-group {
  gap: 12px;
}

.end {
  justify-content: flex-end;
  gap: 8px;
}

.play-button {
  display: flex;
  width: 30px;
  height: 30px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: #fff;
  color: #111;
}

.clock {
  font-size: 12px;
  color: #d1d5db;
}

.zoom-label {
  font-size: 11px;
  color: #9ca3af;
}

.range {
  position: relative;
  width: 112px;
  height: 4px;
  border-radius: 2px;
  background: #3a3a3c;
}

.range.short {
  width: 80px;
}

.range i {
  position: absolute;
  inset: 0 55% 0 0;
  border-radius: 2px;
  background: #0a84ff;
}

.timeline {
  display: flex;
  height: 194px;
  flex-shrink: 0;
  border-top: 1px solid #222a35;
  background: #1c1c1e;
}

.labels {
  width: 84px;
  flex-shrink: 0;
  background: #242426;
  font-size: 11px;
  color: #9ca3af;
}

.label {
  display: flex;
  height: 34px;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid rgb(31 41 55 / 70%);
  padding: 0 12px;
  white-space: nowrap;
}

.label.ruler-label {
  height: 22px;
  font-size: 10px;
}

.label.main {
  height: 70px;
  background: rgb(139 92 246 / 8%);
}

.stack {
  display: flex;
  flex-direction: column;
  line-height: 1.2;
}

.stack small {
  font-size: 10px;
  color: #a78bfa;
}

.c-blue {
  color: #0a84ff;
}

.c-amber {
  color: #f59e0b;
}

.c-violet {
  color: #8b5cf6;
}

.c-green {
  color: #34d399;
}

.tracks {
  position: relative;
  flex: 1;
  overflow: hidden;
}

.ruler {
  position: relative;
  height: 22px;
  border-bottom: 1px solid rgb(31 41 55 / 70%);
  font-size: 9px;
  color: #6b7280;
}

.ruler span {
  position: absolute;
  top: 5px;
  padding-left: 4px;
  border-left: 1px solid #3a3a3c;
}

.track {
  position: relative;
  height: 34px;
  border-bottom: 1px solid rgb(31 41 55 / 70%);
}

.track.main {
  height: 70px;
  background: rgb(139 92 246 / 5%);
}

.subtitle {
  position: absolute;
  top: 5px;
  display: flex;
  height: 24px;
  align-items: center;
  overflow: hidden;
  border: 1px solid rgb(10 132 255 / 50%);
  border-radius: 4px;
  background: rgb(10 132 255 / 20%);
  padding: 0 5px;
  font-size: 10px;
  color: #dbeafe;
  white-space: nowrap;
}

.overlay,
.sound {
  position: absolute;
  top: 4px;
  bottom: 4px;
  display: flex;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  border-radius: 6px;
  padding: 0 6px;
  font-size: 10px;
  white-space: nowrap;
}

.overlay.text {
  border: 1px solid rgb(245 158 11 / 55%);
  background: rgb(245 158 11 / 18%);
  color: #fef3c7;
}

.sound {
  border: 1px solid rgb(52 211 153 / 50%);
  background: rgb(52 211 153 / 14%);
  color: #d1fae5;
}

.clip {
  position: absolute;
  top: 4px;
  bottom: 4px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid rgb(139 92 246 / 50%);
  border-radius: 6px;
  background: rgb(139 92 246 / 15%);
}

.clip.selected {
  border-color: #c4b5fd;
  box-shadow: 0 0 0 1px #c4b5fd;
}

.clip-head {
  display: flex;
  height: 22px;
  flex-shrink: 0;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  background: rgb(139 92 246 / 20%);
  padding: 0 6px;
  font-size: 11px;
  color: #ede9fe;
  white-space: nowrap;
}

.clip-name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.wave {
  flex: 1;
  margin: 4px;
  background: repeating-linear-gradient(
    90deg,
    rgb(196 181 253 / 55%) 0 1px,
    transparent 1px 3px
  );
  mask-image: linear-gradient(
    180deg,
    transparent 10%,
    #000 40%,
    #000 60%,
    transparent 90%
  );
  opacity: 0.8;
}

.drop-slot {
  position: absolute;
  top: 4px;
  bottom: 4px;
  width: 30%;
}

.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  z-index: 20;
  width: 1px;
  background: #ff453a;
  transition: left 250ms linear;
}

.playhead::before {
  position: absolute;
  top: 0;
  left: -5px;
  width: 11px;
  height: 11px;
  border-radius: 0 0 3px 3px;
  background: #ff453a;
  content: "";
}

.playhead.moving {
  transition: left 250ms linear;
}
</style>
