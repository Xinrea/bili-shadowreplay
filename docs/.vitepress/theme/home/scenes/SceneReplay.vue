<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import Icon from "../Icon.vue";
import Pointer from "../Pointer.vue";
import Thumb from "../Thumb.vue";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

/** Length of the stream so far, in seconds. */
const LENGTH = 7120;

const pad = (n: number) => String(Math.floor(n)).padStart(2, "0");
const clock = (s: number) => `${pad(s / 3600)}:${pad((s % 3600) / 60)}:${pad(s % 60)}`;
const percent = (s: number) => `${(s / LENGTH) * 100}%`;

type Range = { start: number; end: number };
type Peak = Range & { count: number; exists: boolean };

const initialPeaks = (): Peak[] => [
  { start: 1450, end: 1515, count: 412, exists: true },
  { start: 5465, end: 5512, count: 386, exists: false },
  { start: 3500, end: 3541, count: 271, exists: false },
  { start: 6450, end: 6492, count: 244, exists: false },
];

// Danmaku per 30 seconds: a calm stream with four bursts.
const bursts = [0.206, 0.494, 0.77, 0.909];
const heat = Array.from({ length: 150 }, (_, i) => {
  const x = i / 150;
  const noise = 0.18 + 0.12 * Math.abs(Math.sin(i * 12.9898) * 43758.5453 % 1);
  const peak = bursts.reduce((sum, b, k) => sum + (1 - k * 0.14) * Math.exp(-(((x - b) * 110) ** 2)), 0);
  return Math.min(1, noise + peak * 0.85);
});
const THRESHOLD = 0.62;

// Rows fill from the top, like the room's danmaku; all in the default white.
const comments = [
  { text: "这波操作太秀了", lane: 0, delay: 0 },
  { text: "满星了！！！", lane: 1, delay: 2.2 },
  { text: "前方高能", lane: 2, delay: 4.6 },
  { text: "哈哈哈哈哈哈", lane: 3, delay: 1.2 },
  { text: "录下来了吗", lane: 0, delay: 5.4 },
  { text: "切片 man 在哪", lane: 4, delay: 3.4 },
  { text: "8888888", lane: 1, delay: 7 },
  { text: "名场面预定", lane: 3, delay: 6.4 },
  { text: "主播手好稳", lane: 2, delay: 8.2 },
];

const state = reactive({
  time: 5462,
  length: LENGTH,
  ranges: [{ start: 1450, end: 1502 }] as Range[],
  selected: 0,
  peaks: initialPeaks(),
  tab: "ranges" as "ranges" | "clips",
  confirm: false,
  clips: [] as { name: string; progress: number }[],
});

const total = computed(() => state.ranges.reduce((sum, r) => sum + r.end - r.start, 0));
const minutes = (s: number) => `${pad(s / 60)}:${pad(s % 60)}`;
/** Like the generate dialog's formatDuration: "2 分 20 秒". */
const duration = (s: number) =>
  [s >= 3600 && `${Math.floor(s / 3600)} 小时`, s >= 60 && `${Math.floor((s % 3600) / 60)} 分`, `${s % 60} 秒`]
    .filter(Boolean)
    .join(" ");

// The stream keeps going while the demo plays.
let ticker: ReturnType<typeof setInterval> | undefined;
onMounted(() => {
  ticker = setInterval(() => {
    if (!props.active) return;
    state.time += 0.25;
    state.length += 0.25;
  }, 250);
});
onBeforeUnmount(() => clearInterval(ticker));

const root = ref<HTMLElement | null>(null);
const generate = ref<HTMLElement | null>(null);
const confirmButton = ref<HTMLElement | null>(null);
const clipsTab = ref<HTMLElement | null>(null);
const peakButton = (index: number) =>
  root.value?.querySelectorAll(".peak-list button")[index]?.querySelector("b");

function addPeak(index: number) {
  const peak = state.peaks[index];
  peak.exists = true;
  state.ranges.push({ start: peak.start, end: peak.end });
  state.selected = state.ranges.length - 1;
  state.time = peak.start;
}

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.time = 5462;
    state.length = LENGTH;
    state.ranges = [{ start: 1450, end: 1502 }];
    state.selected = 0;
    state.peaks = initialPeaks();
    state.tab = "ranges";
    state.confirm = false;
    state.clips = [];
  },
  async run(s) {
    await s.wait(1200);
    await s.click(peakButton(1), 900);
    addPeak(1);
    await s.wait(900);
    await s.click(peakButton(2));
    addPeak(2);
    await s.wait(900);
    await s.click(generate.value, 800);
    state.confirm = true;
    await s.wait(1300);
    await s.click(confirmButton.value, 700);
    state.confirm = false;
    await s.wait(300);
    await s.click(clipsTab.value, 700);
    state.tab = "clips";
    for (const [index] of state.ranges.entries()) {
      state.clips.unshift({ name: `深渊满星挑战 选区 ${index + 1}`, progress: 0 });
      await s.wait(250);
      for (const step of [0.35, 0.7, 1]) {
        state.clips[0].progress = step;
        await s.wait(260);
      }
    }
    await s.move([560, 300], 900);
    await s.wait(1400);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-dark scene">
    <header class="m-titlebar bar">
      <span class="title">深渊满星挑战，今天必拿</span>
      <span class="room-tag mono">BILIBILI#22603245</span>
      <span class="live-tag"><i />直播中</span>
      <span class="bar-actions">
        <span class="m-btn ghost sm icon"><Icon name="list-checks" :size="16" /></span>
        <span class="m-btn ghost sm icon"><Icon name="maximize" :size="16" /></span>
        <span class="m-btn ghost sm icon on"><Icon name="panel-right-close" :size="16" /></span>
      </span>
    </header>

    <div class="workspace">
      <div class="main">
        <div class="stage">
          <div class="video">
            <Thumb scene="dusk" />
            <div class="comments" aria-hidden="true">
              <span
                v-for="item in comments"
                :key="item.text"
                :style="{
                  top: `${item.lane * 22.4}px`,
                  animationDelay: `-${item.delay}s`,
                }"
                >{{ item.text }}</span
              >
            </div>
          </div>
        </div>

        <section class="timeline">
          <div class="timeline-header">
            <div class="left">
              <span class="control on"><Icon name="message-square-text" :size="15" /></span>
            </div>
            <div class="center">
              <span class="control"><Icon name="pause" :size="15" /></span>
              <span class="control"><Icon name="volume-2" :size="15" /></span>
              <span class="time num">{{ clock(state.time) }} / {{ clock(state.length) }}</span>
              <span class="live-button"><i />直播</span>
            </div>
            <div class="right">
              <span class="action"><Icon name="plus" :size="14" />新建选区</span>
              <span class="action"><span class="diamond" />添加标记</span>
            </div>
          </div>

          <div class="grid">
            <div class="heat-row">
              <span class="heat-label">弹幕热度</span>
              <span
                v-for="(value, index) in heat"
                :key="index"
                class="heat-bar"
                :class="{ hot: value >= THRESHOLD, warm: value >= THRESHOLD - 0.1 && value < THRESHOLD }"
                :style="{ left: `${(index / heat.length) * 100}%`, height: `${value * 28}px` }"
              />
              <span class="threshold" :style="{ bottom: `${THRESHOLD * 28}px` }" />
            </div>
            <div class="ruler num">
              <span v-for="k in 5" :key="k">{{ clock(((k - 1) / 4) * state.length) }}</span>
            </div>
            <div class="track">
              <TransitionGroup name="m-pop">
                <span
                  v-for="(range, index) in state.ranges"
                  :key="range.start"
                  class="range-block"
                  :class="{ selected: index === state.selected }"
                  :style="{ left: percent(range.start), width: `max(36px, ${percent(range.end - range.start)})` }"
                >
                  <span class="range-label">选区 {{ index + 1 }}</span>
                </span>
              </TransitionGroup>
              <span class="marker" :style="{ left: percent(2380) }" />
              <span class="marker" :style="{ left: percent(4620) }" />
            </div>
            <span class="playhead" :style="{ left: `${(state.time / state.length) * 100}%` }" />
          </div>
        </section>
      </div>

      <aside class="inspector">
        <div class="heading">素材检查器</div>
        <div class="tabs">
          <span class="tab" :class="{ active: state.tab === 'ranges' }">选区<b>{{ state.ranges.length }}</b></span>
          <span class="tab">弹幕</span>
          <span class="tab">标记<b>2</b></span>
          <span ref="clipsTab" class="tab" :class="{ active: state.tab === 'clips' }">素材</span>
        </div>

        <div v-if="state.tab === 'ranges'" class="content">
          <div class="summary">
            <span>已选内容</span>
            <strong class="num">{{ minutes(total) }}</strong>
            <span class="count num">{{ state.ranges.length }}/{{ state.ranges.length }}</span>
          </div>
          <TransitionGroup name="m-rise" tag="div" class="ranges">
            <div
              v-for="(range, index) in state.ranges"
              :key="range.start"
              class="range-card"
              :class="{ selected: index === state.selected }"
            >
              <span class="m-check on"><Icon name="check" :size="10" :stroke="3" /></span>
              <span class="range-index">{{ index + 1 }}</span>
              <span class="range-time num">{{ clock(range.start) }} → {{ clock(range.end) }}</span>
              <span class="range-duration num">{{ range.end - range.start }} 秒</span>
            </div>
          </TransitionGroup>

          <section class="recommend">
            <div class="section-heading">
              <div>
                <h3>智能推荐</h3>
                <span>基于弹幕热度发现精彩片段</span>
              </div>
              <span class="link">全部添加</span>
            </div>
            <div class="keywords">
              <span class="field-label">关键词过滤</span>
              <span class="keyword-editor">
                <span class="chip">满星</span><span class="chip">高能</span>
                <span class="placeholder">继续添加…</span>
              </span>
            </div>
            <div class="threshold-field">
              <span class="field-label">峰值阈值 80%</span>
              <span class="slider"><i /></span>
            </div>
            <div class="peak-list">
              <button
                v-for="peak in state.peaks"
                :key="peak.start"
                type="button"
                tabindex="-1"
                :class="{ exists: peak.exists }"
              >
                <span class="num">
                  {{ clock(peak.start) }} → {{ clock(peak.end) }}
                  <small>{{ peak.count }} 条弹幕</small>
                </span>
                <b>{{ peak.exists ? "已存在" : "+ 添加" }}</b>
              </button>
            </div>
          </section>
        </div>

        <div v-else class="content">
          <div class="section-heading">
            <div>
              <h3>已生成素材</h3>
              <span>当前直播间下共 {{ state.clips.length + 4 }} 个素材</span>
            </div>
          </div>
          <TransitionGroup name="m-rise" tag="div" class="clip-list">
            <article v-for="clip in state.clips" :key="clip.name" class="clip-card">
              <div class="clip-preview">
                <Thumb scene="dusk" />
                <span v-if="clip.progress < 1" class="progress"><i :style="{ width: `${clip.progress * 100}%` }" /></span>
              </div>
              <div class="clip-info">
                <span class="clip-name">{{ clip.name }}</span>
                <span v-if="clip.progress < 1" class="clip-state">生成中</span>
                <span v-else class="clip-actions">
                  <Icon name="external-link" :size="13" />
                  <Icon name="trash-2" :size="13" />
                </span>
              </div>
            </article>
          </TransitionGroup>
        </div>

        <footer class="footer">
          <div class="summary-line">
            <span>{{ state.ranges.length }} 个选区</span>
            <strong class="num">合成后约 {{ minutes(total) }}</strong>
          </div>
          <span ref="generate" class="m-btn primary">生成素材</span>
        </footer>
      </aside>
    </div>

    <Transition name="m-fade">
      <div v-if="state.confirm" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-body">
            <h3 class="dialog-title">确认生成素材</h3>
            <p class="dialog-text">每个选区生成一个素材</p>
            <div class="dialog-columns">
              <div class="dialog-column">
                <div>
                  <div class="m-label list-label">选区列表</div>
                  <div class="dialog-ranges">
                    <div v-for="(range, index) in state.ranges" :key="range.start" class="dialog-range">
                      <span class="badge num">{{ index + 1 }}</span>
                      <span>
                        <span class="num">{{ clock(range.start) }} → {{ clock(range.end) }}</span>
                        <small>时长: {{ duration(range.end - range.start) }}</small>
                      </span>
                    </div>
                  </div>
                  <div class="total">总时长: {{ duration(total) }}</div>
                </div>
                <label class="field">
                  <span class="field-name">素材备注（可选）</span>
                  <span class="m-input" />
                </label>
              </div>
              <div class="dialog-column">
                <div class="field">
                  <span class="m-label">素材</span>
                  <span class="m-tabs"><span class="m-tab active">分别生成</span><span class="m-tab">合并生成</span></span>
                  <span class="hint">每个选区单独生成素材，可以放进切片里继续剪辑</span>
                </div>
                <div class="field">
                  <span class="m-label">弹幕</span>
                  <span class="m-tabs"><span class="m-tab active">外挂</span><span class="m-tab">内嵌</span></span>
                  <span class="hint">弹幕和素材分开保存，导出切片或投稿时再选择是否压制</span>
                </div>
                <div class="options">
                  <label class="option">
                    <span class="m-check on"><Icon name="check" :size="10" :stroke="3" /></span>
                    <span>
                      同时新建切片
                      <small>按选区顺序把生成的素材放入一个新切片，便于直接剪辑和投稿</small>
                    </span>
                  </label>
                  <label class="option">
                    <span class="m-check" />
                    <span>修复编码（素材异常时使用）</span>
                  </label>
                </div>
              </div>
            </div>
          </div>
          <div class="m-modal-footer">
            <span class="m-btn">取消</span>
            <span ref="confirmButton" class="m-btn primary">确认生成</span>
          </div>
        </div>
      </div>
    </Transition>

    <Pointer :pointer="pointer" />
  </div>
</template>

<style scoped>
.scene {
  display: flex;
  width: 100%;
  height: 100%;
  flex-direction: column;
  background: #0e1219;
}

.bar {
  padding-left: 88px;
  background: #0e1219;
  border-color: #1d242f;
}

.room-tag {
  border-radius: 5px;
  background: #222a36;
  padding: 2px 6px;
  font-size: 11px;
  color: #8c98aa;
}

.live-tag {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 600;
  color: #ff6b6b;
}

.live-tag i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  animation: pulse 1.6s ease-in-out infinite;
}

@keyframes pulse {
  50% {
    opacity: 0.35;
  }
}

.bar-actions {
  display: flex;
  gap: 4px;
  margin-left: auto;
}

.bar-actions .on {
  color: #65c2ff;
}

.workspace {
  display: flex;
  min-height: 0;
  flex: 1;
}

.main {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
}

.stage {
  display: flex;
  min-height: 0;
  flex: 1;
  align-items: center;
  justify-content: center;
  background: #05070a;
}

.video {
  position: relative;
  height: 100%;
  aspect-ratio: 16 / 9;
  max-width: 100%;
  overflow: hidden;
}

.comments span {
  position: absolute;
  left: 100%;
  white-space: nowrap;
  font-family: "Microsoft YaHei", 微软雅黑, sans-serif;
  font-size: 19px;
  font-weight: bold;
  line-height: 1.125;
  color: #fff;
  opacity: 0.8;
  text-shadow: 1px 0 1px #000, 0 1px 1px #000, 0 -1px 1px #000, -1px 0 1px #000;
  animation: fly 9s linear infinite;
}

@keyframes fly {
  to {
    transform: translateX(calc(-820px - 100%));
  }
}

.timeline {
  flex-shrink: 0;
  border-top: 1px solid #2b3340;
  background: #0e1219;
  padding: 0 16px 14px;
  color: #e9eef8;
}

.timeline-header {
  display: grid;
  height: 40px;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 12px;
}

.left,
.center,
.right {
  display: flex;
  align-items: center;
  gap: 6px;
}

.right {
  justify-content: flex-end;
}

.control {
  display: inline-flex;
  width: 28px;
  height: 28px;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  color: #cbd7e8;
}

.control.on {
  background: #153b5c;
  color: #65c2ff;
}

.time {
  margin-left: 4px;
  font-size: 12px;
  color: #dce5f3;
}

.live-button {
  display: inline-flex;
  height: 18px;
  align-items: center;
  gap: 4px;
  border: 1px solid #435064;
  border-radius: 5px;
  background: #202733;
  padding: 0 5px;
  font-size: 9px;
  font-weight: 600;
  color: #aeb9c9;
}

.live-button i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #ff6b6b;
}

.action {
  display: inline-flex;
  height: 28px;
  align-items: center;
  gap: 5px;
  border: 1px solid #303947;
  border-radius: 7px;
  background: #202733;
  padding: 0 9px;
  font-size: 11px;
  color: #cbd7e8;
}

.diamond {
  width: 8px;
  height: 8px;
  transform: rotate(45deg);
  border: 1px solid #fff1b6;
  background: #ffc857;
}

.grid {
  position: relative;
}

.heat-row {
  position: relative;
  height: 30px;
}

.heat-label {
  position: absolute;
  top: 0;
  left: 0;
  z-index: 1;
  font-size: 9px;
  color: #687589;
}

.heat-bar {
  position: absolute;
  bottom: 0;
  width: 0.45%;
  border-radius: 2px 2px 0 0;
  background: #3a4b63;
}

.heat-bar.warm {
  background: #737fd1;
}

.heat-bar.hot {
  background: #35a7ff;
  box-shadow: 0 0 6px rgb(53 167 255 / 45%);
}

.threshold {
  position: absolute;
  right: 0;
  left: 0;
  border-top: 1px dashed rgb(255 200 87 / 55%);
}

.ruler {
  display: flex;
  justify-content: space-between;
  margin-top: 4px;
  font-size: 9px;
  color: #687589;
}

.track {
  position: relative;
  height: 42px;
  margin-top: 4px;
  border-radius: 7px;
  background:
    linear-gradient(90deg, transparent 24.8%, #202733 25%, transparent 25.2%),
    linear-gradient(90deg, transparent 49.8%, #202733 50%, transparent 50.2%),
    linear-gradient(90deg, transparent 74.8%, #202733 75%, transparent 75.2%), #0d1118;
}

.range-block {
  position: absolute;
  top: 5px;
  display: flex;
  height: 32px;
  align-items: center;
  overflow: hidden;
  border: 1px solid #4db5ff;
  border-radius: 6px;
  background: #168de0cc;
  padding: 0 6px;
  font-size: 10px;
  color: #fff;
  white-space: nowrap;
}

.range-block.selected {
  box-shadow: 0 0 0 2px #0d1118, 0 0 0 4px #9cd7ff;
}

.marker {
  position: absolute;
  top: -5px;
  width: 9px;
  height: 9px;
  transform: translateX(-50%) rotate(45deg);
  border: 1px solid #fff1b6;
  background: #ffc857;
}

.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  z-index: 3;
  width: 1px;
  background: #ff6677;
  transition: left 400ms ease;
}

.playhead::before {
  position: absolute;
  top: -3px;
  left: -4px;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #ff6677;
  content: "";
}

/* Inspector */
.inspector {
  display: flex;
  width: 300px;
  flex-shrink: 0;
  flex-direction: column;
  border-left: 1px solid #2b3340;
  background: #121720;
  color: #dce5f3;
}

.heading {
  padding: 14px 16px 10px;
  font-size: 13px;
  font-weight: 600;
}

.tabs {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 2px;
  margin: 0 14px;
  border-radius: 9px;
  background: #0b0f15;
  padding: 3px;
}

.tab {
  display: flex;
  height: 26px;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border-radius: 7px;
  font-size: 12px;
  color: #8c98aa;
  transition: background-color 200ms ease;
}

.tab.active {
  background: #293341;
  color: #e9eef8;
}

.tab b {
  min-width: 16px;
  border-radius: 999px;
  background: #0a84ff;
  padding: 0 5px;
  font-size: 9px;
  font-weight: 500;
  line-height: 16px;
  text-align: center;
  color: #fff;
}

.content {
  min-height: 0;
  flex: 1;
  overflow: hidden;
  padding: 14px;
}

.summary {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 11px;
  color: #8c98aa;
}

.summary strong {
  font-size: 15px;
  font-weight: 600;
  color: #e9eef8;
}

.summary .count {
  margin-left: auto;
}

.ranges {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 10px;
}

.range-card {
  display: flex;
  height: 36px;
  align-items: center;
  gap: 8px;
  border: 1px solid #29313d;
  border-radius: 8px;
  background: #171c25;
  padding: 0 10px;
  font-size: 11px;
}

.range-card.selected {
  border-color: #168de0;
  background: #153b5c;
}

.range-index {
  width: 12px;
  font-weight: 600;
  color: #65c2ff;
}

.range-time {
  color: #dce5f3;
}

.range-duration {
  margin-left: auto;
  color: #8c98aa;
}

.recommend {
  margin-top: 16px;
  border-top: 1px solid #2b3340;
  padding-top: 14px;
}

.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.section-heading h3 {
  margin: 0;
  font-size: 12px;
  font-weight: 600;
}

.section-heading span {
  font-size: 10px;
  color: #8c98aa;
}

.section-heading .link {
  font-size: 11px;
  color: #65c2ff;
}

.keywords,
.threshold-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-top: 10px;
}

.field-label {
  font-size: 10px;
  color: #8c98aa;
}

.keyword-editor {
  display: flex;
  height: 30px;
  align-items: center;
  gap: 4px;
  border: 1px solid #303947;
  border-radius: 7px;
  background: #0d1118;
  padding: 0 6px;
}

.chip {
  border-radius: 5px;
  background: #153b5c;
  padding: 1px 6px;
  font-size: 11px;
  color: #65c2ff;
}

.keyword-editor .placeholder {
  font-size: 11px;
  color: #687589;
}

.slider {
  position: relative;
  height: 4px;
  border-radius: 2px;
  background: #2b3340;
}

.slider i {
  position: absolute;
  inset: 0 40% 0 0;
  border-radius: 2px;
  background: #0a84ff;
}

.slider i::after {
  position: absolute;
  top: -5px;
  right: -7px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #fff;
  content: "";
}

.peak-list {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-top: 12px;
}

.peak-list button {
  display: flex;
  height: 38px;
  align-items: center;
  justify-content: space-between;
  border: 1px solid #303947;
  border-radius: 8px;
  background: #202733;
  padding: 0 10px;
  font: inherit;
  font-size: 11px;
  color: #dce5f3;
  text-align: left;
}

.peak-list small {
  display: block;
  font-size: 10px;
  color: #8c98aa;
}

.peak-list b {
  font-size: 11px;
  font-weight: 500;
  color: #65c2ff;
}

.peak-list .exists {
  opacity: 0.55;
}

.peak-list .exists b {
  color: #8c98aa;
}

.clip-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
}

.clip-card {
  overflow: hidden;
  border: 1px solid #29313d;
  border-radius: 8px;
  background: #171c25;
}

.clip-preview {
  position: relative;
  height: 74px;
}

.progress {
  position: absolute;
  right: 8px;
  bottom: 8px;
  left: 8px;
  height: 4px;
  overflow: hidden;
  border-radius: 2px;
  background: rgb(0 0 0 / 50%);
}

.progress i {
  display: block;
  height: 100%;
  background: #0a84ff;
  transition: width 240ms ease;
}

.clip-info {
  display: flex;
  height: 32px;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px;
  font-size: 11px;
}

.clip-state {
  color: #65c2ff;
}

.clip-actions {
  display: flex;
  gap: 10px;
  color: #8c98aa;
}

.footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border-top: 1px solid #2b3340;
  padding: 12px 14px;
}

.summary-line {
  display: flex;
  flex-direction: column;
  font-size: 10px;
  color: #8c98aa;
}

.summary-line strong {
  font-size: 12px;
  font-weight: 600;
  color: #e9eef8;
}

/* Generate dialog */
.dialog {
  width: 768px;
}

.dialog-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}

.dialog-text {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--muted);
}

.dialog-columns {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px 24px;
  margin-top: 16px;
}

.dialog-column {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 16px;
}

.field {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
}

.field .m-input {
  width: 100%;
}

.field-name {
  font-size: 13px;
  color: var(--fg-2);
}

.hint {
  font-size: 11px;
  color: var(--muted);
}

.list-label {
  margin: 0;
}

.dialog-ranges {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
}

.dialog-range {
  display: flex;
  align-items: center;
  gap: 12px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--control);
  padding: 7px 12px;
  font-size: 12px;
}

.dialog-range small,
.option small {
  display: block;
  font-size: 11px;
  color: var(--muted);
}

.badge {
  display: flex;
  width: 24px;
  height: 24px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-soft);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-fg);
}

.total {
  margin-top: 10px;
  border-top: 1px solid var(--line);
  padding-top: 10px;
  font-size: 14px;
  font-weight: 600;
}

.options {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.option {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  font-size: 13px;
  color: var(--fg-2);
}

.option .m-check {
  margin-top: 3px;
}
</style>
