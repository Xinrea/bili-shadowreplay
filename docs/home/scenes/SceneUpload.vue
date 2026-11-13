<script setup lang="ts">
import { reactive, ref } from "vue";
import EditorFrame from "../EditorFrame.vue";
import Icon from "../Icon.vue";
import Pointer from "../Pointer.vue";
import RenderSettings from "../RenderSettings.vue";
import Thumb from "../Thumb.vue";
import { BGM, MATERIALS, SUBTITLES, TITLE } from "../project";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

type Status = "draft" | "exporting" | "posting" | "posted";

const DESC = "深渊满星全过程，弹幕和字幕都已压制。";
const TAGS = ["原神", "深境螺旋", "星野Hoshino"];

const state = reactive({
  dialog: false,
  status: "draft" as Status,
  account: "",
  desc: "",
  tags: [] as string[],
  draft: "",
  focus: "" as "" | "desc" | "tags",
});

const root = ref<HTMLElement | null>(null);
const body = ref<HTMLElement | null>(null);
const el = (name: string) => root.value?.querySelector(`[data-ref="${name}"]`);

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.dialog = false;
    state.status = "draft";
    state.account = "";
    state.desc = "";
    state.tags = [];
    state.draft = "";
    state.focus = "";
  },
  async run(s) {
    await s.wait(700);
    // The clip editor opens this project's draft over itself.
    await s.click(el("upload"), 900);
    state.dialog = true;
    await s.wait(500);
    await s.click(el("account"), 700);
    state.account = "星野的切片号";
    await s.wait(200);
    await s.click(el("desc"), 600);
    state.focus = "desc";
    await s.type(DESC, (value) => (state.desc = value), 50);
    state.focus = "";
    await s.scroll(body.value, "end", 600);
    await s.click(el("tags"), 600);
    state.focus = "tags";
    for (const tag of TAGS) {
      await s.type(tag, (value) => (state.draft = value), 70);
      await s.wait(200);
      // Enter turns the text into a tag.
      state.tags.push(tag);
      state.draft = "";
      await s.wait(150);
    }
    state.focus = "";
    await s.wait(300);
    await s.click(el("post"), 700);
    state.status = "exporting";
    await s.scroll(body.value, 0, 500);
    await s.wait(1100);
    state.status = "posting";
    await s.wait(1400);
    state.status = "posted";
    await s.wait(500);
    await s.move(el("bvid"), 700);
    await s.wait(1500);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-dark scene">
    <EditorFrame
      :clips="MATERIALS"
      :texts="[TITLE]"
      :sounds="[BGM]"
      :subtitles="SUBTITLES"
      :time="104.5"
      :playing="false"
      panel="materials"
      :used="[0, 1, 2]"
    />

    <Transition name="m-fade">
      <div v-if="state.dialog" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-header">
            <h3>
              <Icon name="upload" :size="16" class="accent" />
              {{ state.status === "draft" ? "编辑投稿" : "投稿详情" }}
            </h3>
            <span class="m-btn sm icon ghost"><Icon name="x" :size="16" /></span>
          </div>
          <div ref="body" class="m-modal-body body">
            <div v-if="state.status === 'posted'" class="banner success">
              <Icon name="circle-check" :size="16" />
              <span>已投稿，B 站稿件审核通过后即可观看。</span>
              <b data-ref="bvid">BV1Zr4y1N7Sx <Icon name="external-link" :size="12" /></b>
            </div>
            <div v-else-if="state.status !== 'draft'" class="banner accent">
              <Icon name="loader-circle" :size="16" class="m-spin" />
              <span>
                {{ state.status === "exporting" ? "正在生成切片视频，完成后会自动投稿。" : "正在投稿，关闭窗口不会中断。" }}
              </span>
              <span class="m-btn sm"><Icon name="list-checks" :size="14" />查看进度</span>
            </div>

            <div class="columns" :class="{ locked: state.status !== 'draft' }">
              <div class="left">
                <section>
                  <div class="row-head">
                    <span class="m-label">视频封面</span>
                    <span v-if="state.status === 'draft'" class="m-btn sm"><Icon name="pen" :size="12" />编辑封面</span>
                  </div>
                  <div class="cover-box">
                    <Thumb scene="dusk" />
                    <span class="cover-title">满星达成！</span>
                  </div>
                </section>
                <section>
                  <span class="m-label">投稿来源</span>
                  <div class="m-card source">
                    <div class="source-line">
                      <Icon name="clapperboard" :size="16" />
                      <span class="m-badge">切片</span>
                      <b>深渊满星集锦</b>
                    </div>
                    <p class="m-hint">投稿时按切片的时间轴生成视频，压制弹幕。</p>
                    <div class="danmu">
                      <span>
                        <b>压制弹幕</b>
                        <span class="m-hint">与切片导出设置中的弹幕选项同步</span>
                      </span>
                      <span class="m-switch on" />
                    </div>
                  </div>
                </section>
                <section>
                  <span class="m-label">画质与码率</span>
                  <div class="m-card source settings">
                    <RenderSettings />
                    <p class="m-hint">与此切片的导出设置同步。</p>
                  </div>
                </section>
              </div>
              <div class="fields">
                <label>
                  <span class="m-label">标题</span>
                  <span class="m-input">深渊满星集锦</span>
                </label>
                <label>
                  <span class="m-label">视频分区</span>
                  <span class="m-input select">游戏 / 手机游戏<Icon name="chevron-down" :size="14" /></span>
                </label>
                <label>
                  <span class="m-label">投稿账号</span>
                  <span class="m-input select" data-ref="account">
                    <span v-if="state.account">{{ state.account }}</span>
                    <span v-else class="placeholder">选择账号</span>
                    <Icon name="chevron-down" :size="14" />
                  </span>
                </label>
                <label>
                  <span class="m-label">描述</span>
                  <span class="m-input area desc" :class="{ focus: state.focus === 'desc' }" data-ref="desc">
                    <span v-if="state.desc || state.focus === 'desc'">{{ state.desc }}<span v-if="state.focus === 'desc'" class="m-caret" /></span>
                    <span v-else class="placeholder">输入视频描述</span>
                  </span>
                </label>
                <div class="field">
                  <span class="m-label">标签</span>
                  <span class="m-input tags" :class="{ focus: state.focus === 'tags' }" data-ref="tags">
                    <TransitionGroup name="m-pop">
                      <span v-for="tag in state.tags" :key="tag" class="m-tag">{{ tag }}<Icon name="x" :size="12" /></span>
                    </TransitionGroup>
                    <span v-if="state.draft" class="draft">{{ state.draft }}</span>
                    <span v-else-if="!state.tags.length" class="placeholder">输入后按回车添加，可用逗号分隔一次添加多个</span>
                    <span v-if="state.focus === 'tags'" class="m-caret" />
                  </span>
                  <span class="m-hint">输入后按回车添加，点击 × 删除</span>
                </div>
                <label>
                  <span class="m-label">动态</span>
                  <span class="m-input area dynamic"><span class="placeholder">输入动态内容</span></span>
                </label>
              </div>
            </div>
          </div>
          <div class="m-modal-footer footer">
            <span class="m-hint">{{ state.status === "draft" ? "修改会自动保存为草稿" : "" }}</span>
            <span class="buttons">
              <span class="m-btn">关闭</span>
              <span v-if="state.status === 'draft'" class="m-btn primary" data-ref="post">
                <Icon name="upload" :size="14" />投稿
              </span>
            </span>
          </div>
        </div>
      </div>
    </Transition>

    <Pointer :pointer="pointer" />
  </div>
</template>

<style scoped>
.scene {
  width: 100%;
  height: 100%;
}

/* The app's upload dialog: at most 90% of the window, the body scrolls. */
.dialog {
  width: 820px;
  max-height: 90%;
}

.m-modal-header h3 {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.accent {
  color: var(--accent-fg);
}

.body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.banner {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 10px;
  border-radius: 10px;
  padding: 9px 14px;
  font-size: 12px;
}

.banner span:first-of-type {
  flex: 1;
}

.banner.accent {
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  background: var(--accent-soft);
  color: var(--accent-fg);
}

.banner.success {
  border: 1px solid color-mix(in srgb, var(--success) 40%, transparent);
  background: var(--success-soft);
  color: var(--success-fg);
}

.banner b {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-weight: 500;
}

.columns {
  display: grid;
  flex-shrink: 0;
  grid-template-columns: 280px minmax(0, 1fr);
  gap: 20px;
}

/* Posting: the form can no longer be changed. */
.columns.locked .fields,
.columns.locked .settings {
  opacity: 0.7;
}

.left {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.left section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.row-head {
  display: flex;
  height: 26px;
  align-items: center;
  justify-content: space-between;
}

.cover-box {
  position: relative;
  aspect-ratio: 16 / 9;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 10px;
}

.cover-title {
  position: absolute;
  top: 14%;
  left: 50%;
  translate: -50% 0;
  font-size: 24px;
  font-weight: 800;
  font-style: italic;
  color: #ffd43b;
  white-space: nowrap;
  -webkit-text-stroke: 5px #c92a2a;
  paint-order: stroke;
}

.source {
  display: flex;
  flex-direction: column;
  gap: 8px;
  background: color-mix(in srgb, var(--control) 50%, transparent);
  padding: 12px;
}

.source-line {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--muted);
}

.source-line b {
  font-size: 12px;
  font-weight: 500;
  color: var(--fg);
}

.source .m-hint {
  margin: 0;
}

.danmu {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  border-top: 1px solid var(--line);
  padding-top: 10px;
}

.danmu b {
  display: block;
  font-size: 12px;
  font-weight: 500;
}

.fields {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.fields label,
.fields .field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.desc {
  min-height: 72px;
}

.dynamic {
  min-height: 54px;
}

.footer {
  justify-content: space-between;
}

.buttons {
  display: flex;
  gap: 8px;
}
</style>
