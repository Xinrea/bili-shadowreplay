<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, type Component } from "vue";
import { withBase } from "vitepress";
import Icon from "./Icon.vue";
import PlatformIcon from "./PlatformIcon.vue";
import SceneRooms from "./scenes/SceneRooms.vue";
import SceneReplay from "./scenes/SceneReplay.vue";
import SceneEditor from "./scenes/SceneEditor.vue";
import SceneSubtitles from "./scenes/SceneSubtitles.vue";
import SceneUpload from "./scenes/SceneUpload.vue";
import "./mock.css";

const RELEASES = "https://github.com/Xinrea/bili-shadowreplay/releases";
const REPO = "https://github.com/Xinrea/bili-shadowreplay";

/** The workflow, in order: each step plays its own scripted window. */
const steps: { title: string; text: string; scene: Component }[] = [
  {
    title: "添加直播间",
    text: "填入房间号或链接即可添加。开播自动录制，下播自动整理成录播。",
    scene: SceneRooms,
  },
  {
    title: "回放与选区",
    text: "直播没结束也能拖回任意时刻。弹幕热度标出高光，一键加入选区并生成素材。",
    scene: SceneReplay,
  },
  {
    title: "切片编辑",
    text: "把素材拖进时间轴，叠加画中画、文字、贴纸和音乐，弹幕可一起压制。",
    scene: SceneEditor,
  },
  {
    title: "AI 字幕",
    text: "PowerLive 参考热词和上下文识别主播名、游戏名，识别后自动校对同音字。",
    scene: SceneSubtitles,
  },
  {
    title: "投稿",
    text: "切片剪完直接投稿到 B 站。生成视频、上传和发布都在后台完成。",
    scene: SceneUpload,
  },
];

const current = ref(0);
const runs = ref(0);
const progress = ref({ step: -1, duration: 0 });
const inView = ref(false);
/** Browsers throttle timers in hidden tabs, so the demo waits for the tab to come back. */
const pageVisible = ref(true);
const playing = computed(() => inView.value && pageVisible.value);
/** After the visitor picks a step, stop moving on by ourselves. */
const autoplay = ref(true);

let advanceTimer: ReturnType<typeof setTimeout> | undefined;

function select(index: number) {
  clearTimeout(advanceTimer);
  autoplay.value = false;
  if (index === current.value) runs.value++;
  current.value = index;
}

function onStart(index: number, duration: number) {
  progress.value = { step: -1, duration: 0 };
  // Restart the bar's transition on the next frame.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => (progress.value = { step: index, duration })),
  );
}

function onDone(index: number) {
  if (!autoplay.value || index !== current.value) return;
  advanceTimer = setTimeout(() => {
    if (autoplay.value && playing.value) current.value = (current.value + 1) % steps.length;
  }, 1400);
}

function onKeydown(event: KeyboardEvent) {
  const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  if (!delta) return;
  event.preventDefault();
  const next = (current.value + delta + steps.length) % steps.length;
  select(next);
  (event.currentTarget as HTMLElement)
    .querySelectorAll<HTMLElement>("[role=tab]")
    [next]?.focus();
}

// The demo windows are drawn at a fixed size and scaled to fit.
const DESIGN_WIDTH = 1120;
const DESIGN_HEIGHT = 680;
const stage = ref<HTMLElement | null>(null);
// Until the first measurement the canvas stays hidden; the stage's own size comes from CSS.
const scale = ref(0);
const canvasStyle = computed(() => ({
  width: `${DESIGN_WIDTH}px`,
  height: `${DESIGN_HEIGHT}px`,
  transform: `scale(${scale.value})`,
}));

let resizeObserver: ResizeObserver | undefined;
let intersectionObserver: IntersectionObserver | undefined;

function onVisibilityChange() {
  pageVisible.value = document.visibilityState === "visible";
  if (pageVisible.value) runs.value++;
}

onMounted(() => {
  document.addEventListener("visibilitychange", onVisibilityChange);
  pageVisible.value = document.visibilityState === "visible";
  if (!stage.value) return;
  resizeObserver = new ResizeObserver(([entry]) => {
    scale.value = entry.contentRect.width / DESIGN_WIDTH;
  });
  resizeObserver.observe(stage.value);
  intersectionObserver = new IntersectionObserver(
    ([entry]) => {
      const wasInView = inView.value;
      inView.value = entry.isIntersecting;
      // Coming back to a finished step: carry on with the tour.
      if (inView.value && !wasInView && autoplay.value) runs.value++;
    },
    { threshold: 0.35 },
  );
  intersectionObserver.observe(stage.value);
});

onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", onVisibilityChange);
  clearTimeout(advanceTimer);
  resizeObserver?.disconnect();
  intersectionObserver?.disconnect();
});

// Viewer comments drifting above the title, in three lanes.
const danmaku = [
  { text: "录播在哪里", lane: 0, delay: 0, speed: 26 },
  { text: "前方高能", lane: 1, delay: 6, speed: 22, tone: "pink" },
  { text: "这段必须切", lane: 2, delay: 2, speed: 30 },
  { text: "刚才那段再放一遍", lane: 1, delay: 15, speed: 28 },
  { text: "名场面 +1", lane: 2, delay: 13, speed: 24, tone: "blue" },
  { text: "已录，下播后见", lane: 0, delay: 14, speed: 27 },
  { text: "切片 man 速来", lane: 0, delay: 7, speed: 25, tone: "pink" },
  { text: "01:42:13 笑死", lane: 2, delay: 21, speed: 29 },
  { text: "求个 BV 号", lane: 1, delay: 0, speed: 23 },
  { text: "字幕辛苦了", lane: 0, delay: 20, speed: 26, tone: "blue" },
];

const extras = [
  {
    icon: "message-square-text",
    title: "弹幕完整保存",
    text: "录制弹幕和醒目留言，可以导出 ASS，也可以压制进切片。",
  },
  {
    icon: "captions",
    title: "多种字幕来源",
    text: "本地 Whisper 模型、在线 Whisper 接口和 PowerLive 任选，字幕样式可调。",
  },
  {
    icon: "bot",
    title: "AI 助手",
    text: "用对话查找录播、分析直播内容和生成切片，也能作为 MCP Server 接入其他工具。",
  },
  {
    icon: "webhook",
    title: "Webhook",
    text: "开播、下播、录制完成和投稿等事件推送到你自己的服务。",
  },
  {
    icon: "container",
    title: "服务器部署",
    text: "Docker 部署后用浏览器访问完整界面，长期值守录制。",
  },
  {
    icon: "image",
    title: "封面编辑",
    text: "从视频画面中选取封面，叠加文字和贴纸，投稿时一起上传。",
  },
];

const platformList = [
  { id: "bilibili", label: "哔哩哔哩" },
  { id: "douyin", label: "抖音" },
  { id: "douyu", label: "斗鱼" },
  { id: "huya", label: "虎牙" },
  { id: "kuaishou", label: "快手" },
  { id: "tiktok", label: "TikTok" },
  { id: "twitch", label: "Twitch" },
  { id: "youtube", label: "YouTube" },
];

const dockerCommand = `docker run -d -p 3000:3000 \\
  -v $DATA_DIR:/app/data \\
  -v $CACHE_DIR:/app/cache \\
  -v $OUTPUT_DIR:/app/output \\
  --name bili-shadowreplay \\
  ghcr.io/xinrea/bili-shadowreplay:latest`;

const copied = shallowRef(false);
async function copyDocker() {
  try {
    await navigator.clipboard.writeText(dockerCommand);
    copied.value = true;
    setTimeout(() => (copied.value = false), 1600);
  } catch {
    // Clipboard access can be refused; the command stays selectable.
  }
}
</script>

<template>
  <div class="sr-home">
    <section class="hero">
      <div class="danmaku" aria-hidden="true">
        <span
          v-for="item in danmaku"
          :key="item.text"
          class="comment"
          :class="item.tone"
          :style="{
            top: `${20 + item.lane * 30}px`,
            animationDuration: `${item.speed}s`,
            animationDelay: `-${item.delay}s`,
          }"
          >{{ item.text }}</span
        >
      </div>
      <div class="wrap hero-grid">
        <div class="title-box">
          <h1 class="headline">
            <img class="logo" :src="withBase('/images/icon.svg')" alt="" />
            <span class="name">
              <span class="name-top">BiliBili</span>
              <span class="name-main">ShadowReplay</span>
            </span>
          </h1>
        </div>
        <div class="hero-side">
          <p class="lead">
            ShadowReplay 在后台录制你关注的直播间。边看边圈出精彩片段，剪辑、加字幕、投稿到
            B 站，都在同一个应用里完成。
          </p>
          <div class="actions">
            <a class="btn primary" :href="RELEASES" target="_blank" rel="noreferrer">
              <Icon name="download" :size="16" />下载桌面版
            </a>
            <a class="btn" :href="withBase('/getting-started/installation/desktop')">阅读文档</a>
          </div>
          <p class="fine">开源免费，支持 Windows、macOS、Linux，也可以用 Docker 部署。</p>
        </div>
      </div>
    </section>

    <section class="tour" aria-label="功能演示">
      <div class="wrap">
        <div class="steps" role="tablist" aria-label="工作流程" @keydown="onKeydown">
          <button
            v-for="(step, index) in steps"
            :key="step.title"
            type="button"
            role="tab"
            class="step"
            :class="{ active: index === current }"
            :aria-selected="index === current"
            :tabindex="index === current ? 0 : -1"
            @click="select(index)"
          >
            <span class="bar">
              <span
                class="fill"
                :style="
                  progress.step === index
                    ? { transform: 'scaleX(1)', transitionDuration: `${progress.duration}ms` }
                    : index === current
                      ? {}
                      : { transform: 'scaleX(0)', transitionDuration: '0ms' }
                "
              />
            </span>
            <span class="step-head">
              <span class="step-index">{{ index + 1 }}</span>
              <span class="step-title">{{ step.title }}</span>
            </span>
            <span class="step-text">{{ step.text }}</span>
          </button>
        </div>

        <div ref="stage" class="stage" :class="{ measured: scale > 0 }">
          <div class="canvas" :style="canvasStyle">
            <div class="window" :class="{ dark: current === 1 || current === 2 || current === 3 }">
              <span class="lights" aria-hidden="true"><i /><i /><i /></span>
              <template v-for="(step, index) in steps" :key="step.title">
                <div
                  v-show="index === current"
                  class="scene-slot"
                  role="tabpanel"
                  :aria-label="step.title"
                >
                  <component
                    :is="step.scene"
                    :key="index === current ? runs : -1"
                    :active="index === current && playing"
                    @start="(duration: number) => onStart(index, duration)"
                    @done="onDone(index)"
                  />
                </div>
              </template>
            </div>
          </div>
        </div>
        <p class="stage-caption">{{ steps[current].text }}</p>
      </div>
    </section>

    <section class="platforms">
      <div class="wrap platforms-row">
        <h2>支持 8 个直播平台</h2>
        <ul>
          <li v-for="item in platformList" :key="item.id">
            <PlatformIcon :id="item.id" :size="20" />{{ item.label }}
          </li>
        </ul>
      </div>
    </section>

    <section class="extras">
      <div class="wrap extras-grid">
        <div class="extras-intro">
          <h2>录播之外</h2>
          <p>从录制到投稿之间的琐事，都可以交给它。</p>
          <a :href="withBase('/usage/features/workflow')">查看完整功能说明</a>
        </div>
        <ul class="extras-list">
          <li v-for="item in extras" :key="item.title">
            <Icon :name="item.icon" :size="20" :stroke="1.75" />
            <h3>{{ item.title }}</h3>
            <p>{{ item.text }}</p>
          </li>
        </ul>
      </div>
    </section>

    <section class="install">
      <div class="wrap install-grid">
        <div class="install-card">
          <h2>桌面版</h2>
          <p>在 GitHub Releases 下载对应系统的安装包。</p>
          <ul class="os-list">
            <li><b>Windows</b><span>Nvidia GPU 加速版与 CPU 版可选</span></li>
            <li><b>macOS</b><span>内置 Metal GPU 加速，需先安装 FFmpeg</span></li>
            <li><b>Linux</b><span>使用 CPU 识别字幕</span></li>
          </ul>
          <div class="actions">
            <a class="btn primary" :href="RELEASES" target="_blank" rel="noreferrer">
              <Icon name="download" :size="16" />前往下载
            </a>
            <a class="btn" :href="withBase('/getting-started/installation/desktop')">安装说明</a>
          </div>
        </div>
        <div class="install-card">
          <h2>Docker</h2>
          <p>部署到 NAS 或服务器，通过浏览器访问 3000 端口。</p>
          <div class="code">
            <pre><code>{{ dockerCommand }}</code></pre>
            <button type="button" class="copy" @click="copyDocker">
              <Icon :name="copied ? 'check' : 'copy'" :size="14" />{{ copied ? "已复制" : "复制" }}
            </button>
          </div>
          <a class="text-link" :href="withBase('/getting-started/installation/docker')">Docker 部署说明</a>
        </div>
      </div>
    </section>

    <footer class="foot">
      <div class="wrap foot-row">
        <span class="brand">
          <img :src="withBase('/images/icon.svg')" alt="" width="20" height="20" />
          BiliBili ShadowReplay
        </span>
        <span class="foot-links">
          <a :href="REPO" target="_blank" rel="noreferrer">GitHub</a>
          <a :href="`${REPO}/issues`" target="_blank" rel="noreferrer">问题反馈</a>
          <span>MIT License</span>
        </span>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.sr-home {
  --h-bg: #f3f5f8;
  --h-raised: #ffffff;
  --h-ink: #171c26;
  --h-ink-2: #475263;
  --h-muted: #6b7687;
  --h-line: #e1e5ec;
  --h-accent: #0a84ff;
  --h-accent-ink: #0066d6;
  --h-pink: #fb7299;
  --h-shadow: 0 50px 100px -30px rgb(23 28 38 / 35%), 0 30px 60px -40px rgb(23 28 38 / 30%);
  background: var(--h-bg);
  color: var(--h-ink);
  font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans GB",
    "Microsoft YaHei", "Noto Sans CJK SC", "Segoe UI", sans-serif;
}

.dark .sr-home {
  --h-bg: #0e1219;
  --h-raised: #171c25;
  --h-ink: #e9eef8;
  --h-ink-2: #b6c0cf;
  --h-muted: #8c98aa;
  --h-line: #2b3340;
  --h-accent-ink: #65c2ff;
  --h-shadow: 0 50px 100px -30px rgb(0 0 0 / 70%), 0 0 0 1px rgb(255 255 255 / 6%);
}

.wrap {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 24px;
}

a {
  color: inherit;
  text-decoration: none;
}

h1,
h2,
h3,
p {
  margin: 0;
}

/* Hero */
.hero {
  position: relative;
  overflow: hidden;
  padding: 116px 0 72px;
}

.danmaku {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.comment {
  position: absolute;
  left: 100%;
  white-space: nowrap;
  font-size: 18px;
  font-weight: 600;
  line-height: 24px;
  color: var(--h-ink);
  opacity: 0.16;
  animation: fly linear infinite;
  will-change: transform;
}

.comment.pink {
  color: var(--h-pink);
  opacity: 0.55;
}

.comment.blue {
  color: var(--h-accent);
  opacity: 0.4;
}

@keyframes fly {
  to {
    transform: translateX(calc(-100vw - 100%));
  }
}

.hero-grid {
  position: relative;
  display: grid;
  gap: 32px;
}

/* The name scales with its column so "ShadowReplay" never overflows it. */
.title-box {
  container-type: inline-size;
}

.headline {
  display: flex;
  align-items: center;
  gap: 3cqw;
  color: var(--h-ink);
}

.logo {
  width: clamp(48px, 18cqw, 128px);
  height: auto;
  flex-shrink: 0;
  filter: drop-shadow(0 14px 22px rgb(23 28 38 / 28%));
}

.name {
  display: flex;
  min-width: 0;
  flex-direction: column;
}

.name-top {
  font-size: clamp(14px, 4cqw, 28px);
  font-weight: 600;
  line-height: 1.2;
  color: var(--h-muted);
}

.name-main {
  font-size: min(10.5cqw, 76px);
  font-weight: 800;
  line-height: 1.05;
  letter-spacing: -0.03em;
  white-space: nowrap;
}

.lead {
  max-width: 34em;
  font-size: 17px;
  line-height: 1.75;
  color: var(--h-ink-2);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 28px;
}

.btn {
  display: inline-flex;
  height: 44px;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--h-line);
  border-radius: 10px;
  background: var(--h-raised);
  padding: 0 20px;
  font-size: 15px;
  font-weight: 600;
  color: var(--h-ink);
  transition: border-color 150ms ease, background-color 150ms ease;
}

.btn:hover {
  border-color: var(--h-muted);
}

.btn.primary {
  border-color: transparent;
  background: var(--h-accent);
  color: #fff;
}

.btn.primary:hover {
  background: #0077ec;
}

.btn:focus-visible,
.step:focus-visible,
.copy:focus-visible,
a:focus-visible {
  outline: 2px solid var(--h-accent);
  outline-offset: 2px;
}

.fine {
  margin-top: 16px;
  font-size: 13px;
  color: var(--h-muted);
}

@media (min-width: 960px) {
  .hero {
    padding: 128px 0 88px;
  }

  .hero-grid {
    grid-template-columns: 1.25fr 1fr;
    align-items: center;
    gap: 64px;
  }

}

/* Tour */
.tour {
  padding-bottom: 96px;
}

.steps {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 20px;
  margin-bottom: 28px;
}

.step {
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 0;
  background: none;
  padding: 0 0 4px;
  color: var(--h-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.bar {
  position: relative;
  height: 2px;
  overflow: hidden;
  border-radius: 2px;
  background: var(--h-line);
}

.fill {
  position: absolute;
  inset: 0;
  background: var(--h-ink);
  transform: scaleX(0);
  transform-origin: left;
  transition-property: transform;
  transition-timing-function: linear;
}

.step.active .fill {
  background: var(--h-accent);
}

.step-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-top: 8px;
}

.step-index {
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}

.step-title {
  font-size: 16px;
  font-weight: 600;
}

.step-text {
  font-size: 13px;
  line-height: 1.65;
}

.step:hover,
.step.active {
  color: var(--h-ink);
}

.step.active .step-index {
  color: var(--h-accent);
}

.step:not(.active) .step-text {
  opacity: 0.8;
}

.stage {
  position: relative;
  aspect-ratio: 1120 / 680;
}

.stage:not(.measured) .canvas {
  visibility: hidden;
}

.canvas {
  position: absolute;
  top: 0;
  left: 0;
  transform-origin: top left;
}

.window {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  border: 1px solid rgb(0 0 0 / 10%);
  border-radius: 14px;
  background: #f3f5f8;
  box-shadow: var(--h-shadow);
}

.dark .window,
.window.dark {
  border-color: rgb(255 255 255 / 9%);
  background: #0e1219;
}

.lights {
  position: absolute;
  top: 14px;
  left: 16px;
  z-index: 60;
  display: flex;
  gap: 8px;
}

.lights i {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #ff5f57;
  box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 15%);
}

.lights i:nth-child(2) {
  background: #febc2e;
}

.lights i:nth-child(3) {
  background: #28c840;
}

.scene-slot {
  position: absolute;
  inset: 0;
}

.stage-caption {
  display: none;
}

@media (max-width: 767px) {
  .comment {
    font-size: 14px;
  }

  .steps {
    display: flex;
    gap: 6px;
    margin: 0 -24px 20px;
    overflow-x: auto;
    padding: 0 24px;
    scrollbar-width: none;
  }

  .step {
    flex-shrink: 0;
    padding: 0;
  }

  .bar,
  .step-text {
    display: none;
  }

  .step-head {
    margin: 0;
    border: 1px solid var(--h-line);
    border-radius: 999px;
    padding: 6px 12px;
  }

  .step.active .step-head {
    border-color: var(--h-ink);
  }

  .step-title {
    font-size: 14px;
  }

  .window {
    border-radius: 18px;
  }

  .stage-caption {
    display: block;
    margin-top: 20px;
    font-size: 14px;
    line-height: 1.7;
    color: var(--h-ink-2);
  }
}

/* Platforms */
.platforms {
  border-top: 1px solid var(--h-line);
  padding: 40px 0;
}

.platforms-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 20px 40px;
}

.platforms h2 {
  font-size: 15px;
  font-weight: 600;
  color: var(--h-ink-2);
}

.platforms ul {
  display: flex;
  flex-wrap: wrap;
  gap: 14px 28px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.platforms li {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  color: var(--h-ink-2);
}

/* Extras */
.extras {
  border-top: 1px solid var(--h-line);
  padding: 96px 0;
}

.extras-grid {
  display: grid;
  gap: 48px;
}

.extras h2,
.install h2 {
  font-size: 32px;
  font-weight: 700;
  letter-spacing: -0.01em;
}

.extras-intro p {
  max-width: 22em;
  margin-top: 14px;
  font-size: 15px;
  line-height: 1.75;
  color: var(--h-ink-2);
}

.extras-intro a,
.text-link {
  display: inline-block;
  margin-top: 12px;
  color: var(--h-accent-ink);
  font-weight: 500;
}

.extras-intro a:hover,
.text-link:hover {
  text-decoration: underline;
}

.extras-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 40px 36px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.extras-list li {
  color: var(--h-accent);
}

.extras-list h3 {
  margin-top: 14px;
  font-size: 16px;
  font-weight: 600;
  color: var(--h-ink);
}

.extras-list p {
  margin-top: 6px;
  font-size: 14px;
  line-height: 1.7;
  color: var(--h-ink-2);
}

@media (min-width: 960px) {
  .extras-grid {
    grid-template-columns: 1fr 2.4fr;
    gap: 64px;
  }
}

/* Install */
.install {
  padding: 0 0 96px;
}

.install-grid {
  display: grid;
  gap: 20px;
}

.install-card {
  border: 1px solid var(--h-line);
  border-radius: 16px;
  background: var(--h-raised);
  padding: 32px;
}

.install h2 {
  font-size: 24px;
}

.install-card > p {
  margin-top: 10px;
  font-size: 14px;
  line-height: 1.7;
  color: var(--h-ink-2);
}

.os-list {
  display: flex;
  flex-direction: column;
  margin: 18px 0 0;
  padding: 0;
  list-style: none;
}

.os-list li {
  display: flex;
  align-items: baseline;
  gap: 16px;
  border-top: 1px solid var(--h-line);
  padding: 10px 0;
  font-size: 14px;
}

.os-list b {
  width: 72px;
  flex-shrink: 0;
  font-weight: 600;
}

.os-list span {
  color: var(--h-ink-2);
}

.install-card .actions {
  margin-top: 24px;
}

.code {
  position: relative;
  margin-top: 20px;
  border-radius: 10px;
  background: #0e1219;
}

.dark .code {
  background: #0a0d12;
  box-shadow: inset 0 0 0 1px var(--h-line);
}

.code pre {
  margin: 0;
  overflow-x: auto;
  padding: 16px 18px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12.5px;
  line-height: 1.7;
  color: #dce5f3;
}

.copy {
  position: absolute;
  top: 10px;
  right: 10px;
  display: inline-flex;
  height: 28px;
  align-items: center;
  gap: 6px;
  border: 1px solid rgb(255 255 255 / 14%);
  border-radius: 7px;
  background: #202733;
  padding: 0 10px;
  font: inherit;
  font-size: 12px;
  color: #dce5f3;
  cursor: pointer;
}

.text-link {
  margin-top: 16px;
  font-size: 14px;
}

@media (min-width: 960px) {
  .install-grid {
    grid-template-columns: 1fr 1.3fr;
  }
}

/* Footer */
.foot {
  border-top: 1px solid var(--h-line);
  padding: 28px 0 40px;
  font-size: 13px;
  color: var(--h-muted);
}

.foot-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  color: var(--h-ink-2);
}

.foot-links {
  display: flex;
  gap: 20px;
}

.foot-links a:hover {
  color: var(--h-ink);
}

@media (prefers-reduced-motion: reduce) {
  .comment {
    display: none;
  }
}
</style>

<style>
/* The home page sets its own background; extend it behind the nav and past the footer. */
:root:has(.sr-home) {
  --vp-c-bg: #f3f5f8;
  --vp-nav-bg-color: #f3f5f8;
}

.dark:has(.sr-home) {
  --vp-c-bg: #0e1219;
  --vp-nav-bg-color: #0e1219;
}

.VPHome:has(.sr-home) {
  margin-bottom: 0 !important;
}
</style>
