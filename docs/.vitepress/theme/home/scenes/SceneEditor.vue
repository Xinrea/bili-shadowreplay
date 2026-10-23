<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import EditorFrame, { type Clip, type Span } from "../EditorFrame.vue";
import Pointer from "../Pointer.vue";
import Thumb from "../Thumb.vue";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

const materials: Clip[] = [
  { start: 0, length: 52, label: "深渊满星挑战 选区 1", scene: "dusk" },
  { start: 52, length: 47, label: "深渊满星挑战 选区 2", scene: "forest" },
  { start: 99, length: 41, label: "深渊满星挑战 选区 3", scene: "sea" },
];
const BGM: Span = { start: 0, length: 140, label: "轻快 BGM.mp3" };
const TITLE: Span = { start: 48, length: 8, label: "满星达成！" };

const state = reactive({
  clips: [materials[0]] as Clip[],
  texts: [] as Span[],
  time: 0,
  playing: false,
  panel: "materials" as "materials" | "text",
  danmu: false,
  used: [0],
  dragging: null as Clip | null,
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
    state.danmu = false;
    state.used = [0];
    state.dragging = null;
  },
  async run(s) {
    await s.wait(700);
    for (const index of [1, 2]) {
      await s.move(el(`bin-${index}`), 800);
      pointer.down = true;
      state.dragging = materials[index];
      await s.wait(150);
      await s.move(el("drop"), 900);
      pointer.down = false;
      state.dragging = null;
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
    await s.click(el("danmu"), 800);
    state.danmu = true;
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
      :danmu="state.danmu"
      :caption="caption"
      :used="state.used"
    />
    <div
      v-if="state.dragging"
      class="ghost"
      :style="{
        transform: `translate(${pointer.x}px, ${pointer.y}px)`,
        transitionDuration: `${pointer.ms}ms`,
      }"
    >
      <span class="ghost-thumb"><Thumb :scene="state.dragging.scene" /></span>
      {{ state.dragging.label }}
    </div>
    <Pointer :pointer="pointer" />
  </div>
</template>

<style scoped>
.scene {
  width: 100%;
  height: 100%;
}

.ghost {
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
