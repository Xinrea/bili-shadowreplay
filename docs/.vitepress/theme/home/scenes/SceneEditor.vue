<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import EditorFrame, { type Clip, type Span } from "../EditorFrame.vue";
import Icon from "../Icon.vue";
import Pointer from "../Pointer.vue";
import RenderSettings from "../RenderSettings.vue";
import Thumb from "../Thumb.vue";
import { BGM, DANMU_COUNT, MATERIALS as materials, TITLE } from "../project";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

const state = reactive({
  clips: [materials[0]] as Clip[],
  texts: [] as Span[],
  time: 0,
  playing: false,
  panel: "materials" as "materials" | "text",
  used: [0],
  dragging: null as Clip | null,
  /** Where the dragged material would go, shown on the main track. */
  dropAt: null as number | null,
  exportDialog: false,
  exportDanmu: false,
  exporting: false,
});

const caption = computed(() =>
  state.texts.find((text) => state.time >= text.start && state.time < text.start + text.length)?.label,
);

const root = ref<HTMLElement | null>(null);
const el = (name: string) => root.value?.querySelector(`[data-ref="${name}"]`);

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.clips = [materials[0]];
    state.texts = [];
    state.time = 0;
    state.playing = false;
    state.panel = "materials";
    state.used = [0];
    state.dragging = null;
    state.dropAt = null;
    state.exportDialog = false;
    state.exportDanmu = false;
    state.exporting = false;
  },
  async run(s) {
    await s.wait(700);
    for (const index of [1, 2]) {
      await s.move(el(`bin-${index}`), 800);
      pointer.down = true;
      state.dragging = materials[index];
      await s.wait(150);
      await s.move(el("drop"), 900);
      // Past the main track's end, so it is appended there.
      state.dropAt = materials[index].start;
      await s.wait(350);
      pointer.down = false;
      state.dragging = null;
      state.dropAt = null;
      state.clips.push(materials[index]);
      state.used.push(index);
      await s.wait(500);
    }
    // Seek near the end of the first clip, then put a title there.
    const tracks = el("tracks");
    if (tracks) {
      const box = tracks.getBoundingClientRect();
      const rootBox = root.value!.getBoundingClientRect();
      const scale = rootBox.width / root.value!.offsetWidth;
      await s.click([
        (box.left - rootBox.left) / scale + (48 / 150) * (box.width / scale),
        (box.top - rootBox.top) / scale + 10,
      ]);
    } else {
      await s.click(null);
    }
    state.time = 48;
    await s.wait(300);
    await s.click(el("rail-text"));
    state.panel = "text";
    await s.wait(400);
    await s.click(el("style-2"));
    state.texts.push(TITLE);
    await s.wait(700);
    await s.click(el("play"));
    state.playing = true;
    for (let i = 0; i < 24; i++) {
      await s.wait(250);
      state.time += 0.25;
    }
    state.playing = false;
    await s.wait(300);
    // Danmaku burning is an export setting, shared with posting.
    await s.click(el("export"), 800);
    state.exportDialog = true;
    await s.wait(600);
    await s.click(el("export-danmu"), 700);
    state.exportDanmu = true;
    await s.wait(700);
    await s.click(el("export-confirm"), 700);
    state.exportDialog = false;
    state.exporting = true;
    await s.wait(1500);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-dark scene">
    <EditorFrame
      :clips="state.clips"
      :texts="state.texts"
      :sounds="[BGM]"
      :subtitles="[]"
      :time="state.time"
      :playing="state.playing"
      :panel="state.panel"
      :exporting="state.exporting"
      :drop-at="state.dropAt"
      :caption="caption"
      :used="state.used"
    />
    <div
      v-if="state.dragging"
      class="drag-ghost"
      :style="{
        transform: `translate(${pointer.x}px, ${pointer.y}px)`,
        transitionDuration: `${pointer.ms}ms`,
      }"
    >
      <span class="ghost-thumb"><Thumb :scene="state.dragging.scene" /></span>
      {{ state.dragging.label }}
    </div>

    <Transition name="m-fade">
      <div v-if="state.exportDialog" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-header">
            <h3><Icon name="download" :size="16" class="accent" />导出设置</h3>
            <span class="m-btn sm icon ghost"><Icon name="x" :size="16" /></span>
          </div>
          <div class="m-modal-body body">
            <div class="switch-row">
              <span>
                <span class="m-label">压制弹幕</span>
                <span class="m-hint">将素材保存的弹幕压制到视频画面中（{{ DANMU_COUNT }} 条）</span>
              </span>
              <span class="m-switch" :class="{ on: state.exportDanmu }" data-ref="export-danmu" />
            </div>
            <RenderSettings />
            <p class="m-hint">这些设置也用于此切片的投稿。</p>
          </div>
          <div class="m-modal-footer">
            <span class="m-btn">取消</span>
            <span class="m-btn primary" data-ref="export-confirm">
              <Icon name="download" :size="14" />选择位置并导出
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

.dialog {
  width: 420px;
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

.body > .m-hint {
  margin: 0;
}

.switch-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.switch-row .m-label,
.switch-row .m-hint {
  display: block;
}

.switch-row .m-switch {
  margin-top: 2px;
}

.drag-ghost {
  position: absolute;
  top: 0;
  left: 0;
  z-index: 40;
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 6px 0 0 10px;
  border: 1px solid rgb(139 92 246 / 70%);
  border-radius: 8px;
  background: rgb(36 30 58 / 92%);
  padding: 4px 10px 4px 4px;
  font-size: 11px;
  color: #ede9fe;
  white-space: nowrap;
  box-shadow: 0 10px 24px rgb(0 0 0 / 45%);
  pointer-events: none;
  transition-property: transform;
  transition-timing-function: cubic-bezier(0.45, 0.05, 0.25, 1);
}

.ghost-thumb {
  width: 48px;
  height: 27px;
  overflow: hidden;
  border-radius: 4px;
}
</style>
