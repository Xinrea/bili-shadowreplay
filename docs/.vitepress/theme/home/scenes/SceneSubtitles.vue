<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import EditorFrame, { type Span } from "../EditorFrame.vue";
import Icon from "../Icon.vue";
import Pointer from "../Pointer.vue";
import { BGM, MATERIALS as clips, SUBTITLES as lines, TITLE } from "../project";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

const sounds: Span[] = [BGM];
const texts: Span[] = [TITLE];

const GENERATED_CONTEXT = "主播星野Hoshino，直播标题：深渊满星挑战，今天必拿";
const MAX_HOTWORDS = 500;
const GLOBAL_HOTWORDS = ["原神", "米哈游"];

const state = reactive({
  modal: false,
  /** 直播间词库: this room's words, typed into below. */
  hotwords: ["星野Hoshino", "深境螺旋"],
  draft: "",
  focus: "" as "" | "hotwords" | "context",
  context: "",
  contextLoading: false,
  subtitles: [] as Span[],
  recognizing: false,
  time: 0,
  playing: false,
});

const contextLength = computed(() => [...state.context].length);
/** Both dictionaries, merged without duplicates as recognizing does. */
const merged = computed(() => new Set([...state.hotwords, ...GLOBAL_HOTWORDS]).size);

const root = ref<HTMLElement | null>(null);
const body = ref<HTMLElement | null>(null);
const el = (name: string) => root.value?.querySelector(`[data-ref="${name}"]`);

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.modal = false;
    state.hotwords = ["星野Hoshino", "深境螺旋"];
    state.draft = "";
    state.focus = "";
    state.context = "";
    state.contextLoading = false;
    state.subtitles = [];
    state.recognizing = false;
    state.time = 0;
    state.playing = false;
  },
  async run(s) {
    await s.wait(700);
    await s.click(el("ai"), 900);
    state.modal = true;
    state.contextLoading = true;
    await s.wait(900);
    state.contextLoading = false;
    state.context = GENERATED_CONTEXT;
    await s.click(el("hotwords"), 700);
    state.focus = "hotwords";
    for (const word of ["胡桃", "雷电将军"]) {
      await s.type(word, (value) => (state.draft = value), 110);
      await s.wait(250);
      state.hotwords.push(word);
      state.draft = "";
      await s.wait(250);
    }
    // The dialog is taller than the window allows: scroll to the context.
    state.focus = "";
    await s.scroll(body.value, "end", 600);
    await s.click(el("context"), 700);
    state.focus = "context";
    await s.type("，在玩原神", (value) => (state.context = GENERATED_CONTEXT + value), 90);
    await s.wait(500);
    await s.click(el("recognize"), 800);
    state.modal = false;
    state.focus = "";
    state.recognizing = true;
    await s.wait(900);
    for (const line of lines) {
      state.subtitles.push(line);
      await s.wait(260);
    }
    state.recognizing = false;
    await s.wait(400);
    // Jump to the fifth line and play through it.
    await s.click(root.value?.querySelectorAll(".row-index")[4], 800);
    state.time = 56.5;
    await s.wait(300);
    await s.click(el("play"), 700);
    state.playing = true;
    for (let i = 0; i < 14; i++) {
      await s.wait(250);
      state.time += 0.25;
    }
    state.playing = false;
    await s.wait(1400);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-dark scene">
    <EditorFrame
      :clips="clips"
      :texts="texts"
      :sounds="sounds"
      :subtitles="state.subtitles"
      :time="state.time"
      :playing="state.playing"
      panel="materials"
      :recognizing="state.recognizing"
      :used="[0, 1, 2]"
    />

    <Transition name="m-fade">
      <div v-if="state.modal" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-header">
            <h3><Icon name="brain-circuit" :size="16" class="accent" />AI 字幕</h3>
            <span class="m-btn sm icon ghost"><Icon name="x" :size="16" /></span>
          </div>
          <div ref="body" class="m-modal-body body">
            <p class="intro">按素材识别字幕，每个素材只识别一次，结果会缓存并按片段范围放入时间轴。</p>
            <div class="checks">
              <label class="check-row">
                <span class="m-check" />
                <span>覆盖已有字幕的片段<small>默认只为还没有字幕的片段识别</small></span>
              </label>
              <label class="check-row">
                <span class="m-check" />
                <span>忽略缓存，重新识别<small>识别结果不准确，或更换了字幕生成器时使用</small></span>
              </label>
            </div>
            <div class="powerlive">
              <p class="m-hint">
                PowerLive 会参考热词和上下文识别专有名词，并在识别后校对同音字。已识别过的素材直接使用缓存，修改后需勾选「忽略缓存，重新识别」才会生效。
              </p>
              <div>
                <div class="field-head">
                  <span class="m-label">直播间词库</span>
                  <span class="m-hint">B站 · 22603245，仅本直播间使用</span>
                </div>
                <div class="m-input tags" :class="{ focus: state.focus === 'hotwords' }" data-ref="hotwords">
                  <TransitionGroup name="m-pop">
                    <span v-for="word in state.hotwords" :key="word" class="m-tag">
                      {{ word }}<Icon name="x" :size="12" />
                    </span>
                  </TransitionGroup>
                  <span class="draft">{{ state.draft }}</span>
                  <span v-if="state.focus === 'hotwords'" class="m-caret" />
                </div>
                <div class="field-foot">
                  <span class="m-hint">主播名、游戏和角色名等专有名词，不要放常用词</span>
                  <span class="faint num">{{ state.hotwords.length }} / {{ MAX_HOTWORDS }}</span>
                </div>
              </div>
              <div>
                <div class="field-head">
                  <span class="m-label">全局词库</span>
                  <span class="m-hint">所有直播间共用，与主窗口设置同步</span>
                </div>
                <div class="m-input tags">
                  <span v-for="word in GLOBAL_HOTWORDS" :key="word" class="m-tag">
                    {{ word }}<Icon name="x" :size="12" />
                  </span>
                </div>
                <div class="field-foot">
                  <span class="m-hint">主播名、游戏和角色名等专有名词，不要放常用词</span>
                  <span class="faint num">{{ GLOBAL_HOTWORDS.length }} / {{ MAX_HOTWORDS }}</span>
                </div>
              </div>
              <p class="m-hint">
                两份词库自动保存，识别时合并去重，优先使用直播间热词。合并后共 {{ merged }} / {{ MAX_HOTWORDS }} 词。
              </p>
              <div class="length-row">
                <span class="m-label">
                  每条字幕最多字数
                  <span class="m-hint">全局设置，范围 4–50</span>
                </span>
                <span class="length-field">
                  <span class="m-input number num">12</span>
                  <span class="m-hint">字</span>
                </span>
              </div>
              <div>
                <div class="field-head">
                  <span class="m-label">上下文</span>
                  <span class="m-btn sm ghost regen">
                    <Icon
                      :name="state.contextLoading ? 'loader-circle' : 'rotate-ccw'"
                      :size="12"
                      :class="{ spin: state.contextLoading }"
                    />按素材重新生成
                  </span>
                </div>
                <div
                  class="m-input area context"
                  :class="{ focus: state.focus === 'context' }"
                  data-ref="context"
                >
                  <span v-if="state.contextLoading" class="placeholder">正在根据素材生成…</span>
                  <span v-else>{{ state.context }}<span v-if="state.focus === 'context'" class="m-caret" /></span>
                </div>
                <div class="field-foot">
                  <span class="m-hint">只用于这次识别，默认根据主播名和直播标题生成</span>
                  <span class="faint num">{{ contextLength }} / 400</span>
                </div>
              </div>
            </div>
          </div>
          <div class="m-modal-footer">
            <span class="m-btn">取消</span>
            <span class="m-btn primary" data-ref="recognize">开始识别</span>
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

.dialog {
  width: 520px;
  /* The app's .ui-modal: at most 85% of the window, the body scrolls. */
  max-height: 85%;
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
  gap: 14px;
}

.intro {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--fg-2);
}

.checks {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.check-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  font-size: 13px;
  color: var(--fg-2);
}

.check-row .m-check {
  margin-top: 3px;
}

.check-row small {
  display: block;
  font-size: 11px;
  color: var(--muted);
}

.powerlive {
  display: flex;
  flex-direction: column;
  gap: 12px;
  border-top: 1px solid var(--line);
  padding-top: 14px;
}

.powerlive > .m-hint {
  margin: 0;
}

.field-head,
.field-foot {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.field-head {
  margin-bottom: 6px;
  align-items: center;
}

.field-foot {
  margin-top: 4px;
}

.faint {
  font-size: 11px;
  color: var(--faint);
}

.length-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.length-row .m-hint {
  margin-left: 6px;
  font-weight: 400;
}

.length-field {
  display: flex;
  align-items: center;
  gap: 6px;
}

.number {
  width: 64px;
  justify-content: flex-end;
}

.regen {
  margin-right: -6px;
}

.spin {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.context {
  min-height: 72px;
}
</style>
