<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import Icon from "../Icon.vue";
import MainWindow from "../MainWindow.vue";
import PlatformIcon from "../PlatformIcon.vue";
import Pointer from "../Pointer.vue";
import Thumb from "../Thumb.vue";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

type Room = {
  platform: string;
  id: string;
  title: string;
  user: string;
  avatar: string;
  scene: "dusk" | "city" | "studio" | "sea" | "forest";
  live: boolean;
  recording: boolean;
};

const initialRooms = (): Room[] => [
  { platform: "bilibili", id: "21484828", title: "深夜杂谈，聊聊这周的新番", user: "夜航Nocturne", avatar: "#7c5cff", scene: "studio", live: true, recording: true },
  { platform: "douyin", id: "728815460", title: "城市夜景慢直播", user: "阿蓝在路上", avatar: "#22b8cf", scene: "city", live: true, recording: true },
  { platform: "twitch", id: "pixel_harbor", title: "Speedrun practice, any%", user: "pixel_harbor", avatar: "#f59f00", scene: "forest", live: false, recording: false },
  { platform: "huya", id: "660118", title: "海岛生存第 12 天", user: "船长老周", avatar: "#e8590c", scene: "sea", live: true, recording: true },
];

const newRoom: Room = {
  platform: "bilibili",
  id: "22603245",
  title: "深渊满星挑战，今天必拿",
  user: "星野Hoshino",
  avatar: "#f06595",
  scene: "dusk",
  live: true,
  recording: false,
};

const platforms = [
  { id: "bilibili", label: "哔哩哔哩" },
  { id: "douyin", label: "抖音" },
  { id: "douyu", label: "斗鱼" },
  { id: "huya", label: "虎牙" },
  { id: "kuaishou", label: "快手" },
  { id: "tiktok", label: "TikTok" },
  { id: "twitch", label: "Twitch" },
  { id: "youtube", label: "YouTube" },
];

const state = reactive({
  rooms: initialRooms(),
  modal: false,
  platform: "douyin",
  input: "",
  focus: false,
  pressed: "",
});

const liveCount = computed(() => state.rooms.filter((room) => room.live).length);

const root = ref<HTMLElement | null>(null);
const addButton = ref<HTMLElement | null>(null);
const bilibiliTab = ref<HTMLElement | null>(null);
const field = ref<HTMLElement | null>(null);
const confirm = ref<HTMLElement | null>(null);

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.rooms = initialRooms();
    state.modal = false;
    state.platform = "douyin";
    state.input = "";
    state.focus = false;
    state.pressed = "";
  },
  async run(s) {
    await s.wait(700);
    await s.click(addButton.value, 900);
    state.modal = true;
    await s.wait(500);
    await s.click(bilibiliTab.value);
    state.platform = "bilibili";
    await s.wait(300);
    await s.click(field.value, 500);
    state.focus = true;
    await s.type("https://live.bilibili.com/22603245", (value) => (state.input = value), 45);
    await s.wait(400);
    await s.click(confirm.value, 700);
    state.modal = false;
    state.focus = false;
    state.rooms = [...initialRooms(), { ...newRoom }];
    await s.wait(1300);
    state.rooms[4].recording = true;
    await s.wait(500);
    await s.move(root.value?.querySelectorAll(".room")[4]?.querySelector(".play"), 800);
    await s.wait(1600);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-follow scene">
    <MainWindow page="直播间">
      <template #subtitle>
        <span class="status">
          <span class="dot" />{{ liveCount }} 直播中
          <span class="sep">·</span>
          {{ state.rooms.length - liveCount }} 未直播
        </span>
      </template>
      <template #actions>
        <span class="m-input search">
          <Icon name="search" :size="13" />
          <span class="placeholder">搜索直播间...</span>
        </span>
        <span ref="addButton" class="m-btn primary add">
          <Icon name="plus" :size="14" />添加直播间
        </span>
      </template>

      <div class="grid">
        <TransitionGroup name="m-rise">
          <div
            v-for="(room, index) in state.rooms"
            :key="room.id"
            class="m-card room"
          >
            <div class="cover">
              <Thumb :scene="room.scene" :dim="!room.live" />
              <span class="tag id mono">{{ room.platform.toUpperCase() }}#{{ room.id }}</span>
              <span class="tag rec" :class="{ recording: room.recording }">
                <span class="rec-icon" />
                <span v-if="room.recording">录制中</span>
              </span>
              <span v-if="room.live" class="tag state"><span class="dot" />直播进行中</span>
              <span v-else class="tag state off">直播未开始</span>
              <span class="tag more"><Icon name="ellipsis" :size="16" /></span>
            </div>
            <div class="meta">
              <div class="name">
                <PlatformIcon :id="room.platform" :size="16" />
                <h3>{{ room.title }}</h3>
              </div>
              <div class="row">
                <span class="user">
                  <span class="avatar" :style="{ background: room.avatar }">{{ room.user.slice(0, 1) }}</span>
                  {{ room.user }}
                </span>
                <span class="buttons">
                  <span v-if="room.recording" class="m-btn ghost icon play"><Icon name="play" :size="16" /></span>
                  <span class="m-btn ghost icon"><Icon name="history" :size="16" /></span>
                </span>
              </div>
            </div>
            <span v-if="index === 4" class="new-flash" />
          </div>
        </TransitionGroup>
        <div class="add-card">
          <span class="plus"><Icon name="plus" :size="20" /></span>
          <strong>添加新直播间</strong>
          <span>支持 B 站、抖音、斗鱼、虎牙等 8 个平台</span>
        </div>
      </div>
    </MainWindow>

    <Transition name="m-fade">
      <div v-if="state.modal" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-header">添加直播间</div>
          <div class="m-modal-body body">
            <div class="field">
              <span class="m-label">平台</span>
              <div class="tabs">
                <span
                  v-for="item in platforms"
                  :key="item.id"
                  :ref="(el) => { if (item.id === 'bilibili') bilibiliTab = el as HTMLElement }"
                  class="tab"
                  :class="{ active: state.platform === item.id }"
                >
                  <PlatformIcon :id="item.id" :size="20" />
                  <span>{{ item.label }}</span>
                </span>
              </div>
            </div>
            <div class="field">
              <span class="m-label">直播间 ID 或链接</span>
              <span ref="field" class="m-input" :class="{ focus: state.focus }">
                <span v-if="state.input">{{ state.input }}</span>
                <span v-else class="placeholder">请输入直播间 ID 或链接</span>
                <span v-if="state.focus" class="m-caret" />
              </span>
            </div>
          </div>
          <div class="m-modal-footer">
            <span class="m-btn">取消</span>
            <span ref="confirm" class="m-btn primary" :class="{ disabled: !state.input }">添加</span>
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

.status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.status .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success);
}

.sep {
  color: var(--faint);
}

.search {
  width: 176px;
  height: 28px;
  gap: 6px;
  border-radius: 7px;
  padding: 0 8px;
  color: var(--faint);
}

.add {
  height: 28px;
  gap: 4px;
  border-radius: 7px;
  padding: 0 10px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  padding: 20px;
}

.room {
  position: relative;
  padding: 12px;
}

.cover {
  position: relative;
  height: 156px;
  overflow: hidden;
  border-radius: 8px;
  background: var(--sunken);
}

.tag {
  position: absolute;
  display: flex;
  height: 24px;
  align-items: center;
  gap: 4px;
  border-radius: 6px;
  background: rgb(0 0 0 / 50%);
  padding: 0 6px;
  font-size: 11px;
  color: #fff;
  backdrop-filter: blur(4px);
}

.tag.id {
  bottom: 8px;
  left: 8px;
  color: rgb(255 255 255 / 85%);
}

.tag.rec {
  top: 8px;
  left: 8px;
  font-weight: 500;
  transition: background-color 300ms ease;
}

.tag.rec.recording {
  background: var(--danger);
}

.rec-icon {
  width: 10px;
  height: 10px;
  margin: 3px;
  border: 2px solid #fff;
  border-radius: 50%;
}

.tag.recording .rec-icon {
  background: radial-gradient(circle, #fff 0 2px, transparent 2.5px);
}

.tag.state {
  right: 8px;
  bottom: 8px;
  gap: 6px;
  padding: 0 8px;
  font-weight: 500;
}

.tag.state .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #30d158;
}

.tag.state.off {
  font-weight: 400;
  color: rgb(255 255 255 / 80%);
}

.tag.more {
  top: 8px;
  right: 8px;
  width: 28px;
  height: 28px;
  justify-content: center;
  padding: 0;
  border-radius: 7px;
}

.meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 12px;
}

.name {
  display: flex;
  align-items: center;
  gap: 8px;
}

.name h3 {
  margin: 0;
  overflow: hidden;
  font-size: 13px;
  font-weight: 500;
  color: var(--fg);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.user {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--fg-2);
}

.avatar {
  display: flex;
  width: 28px;
  height: 28px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  font-size: 12px;
  font-weight: 600;
  color: #fff;
}

.buttons {
  display: flex;
  gap: 4px;
}

.new-flash {
  position: absolute;
  inset: -1px;
  border: 2px solid var(--accent);
  border-radius: 10px;
  opacity: 0;
  pointer-events: none;
  animation: flash 1.8s ease 0.1s;
}

@keyframes flash {
  20%,
  60% {
    opacity: 1;
  }
}

.add-card {
  display: flex;
  min-height: 236px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px dashed var(--line-strong);
  border-radius: 10px;
  padding: 16px;
}

.add-card .plus {
  display: flex;
  width: 40px;
  height: 40px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent-fg);
}

.add-card strong {
  font-size: 13px;
  font-weight: 500;
  color: var(--accent-fg);
}

.add-card span:last-child {
  font-size: 11px;
  color: var(--muted);
}

.dialog {
  width: 440px;
}

.body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tabs {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 2px;
  border-radius: 9px;
  background: var(--sunken);
  padding: 3px;
}

.tab {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  border-radius: 7px;
  padding: 8px 4px;
  font-size: 11px;
  line-height: 1;
  color: var(--muted);
  transition: background-color 200ms ease, color 200ms ease;
}

.tab.active {
  background: var(--selected);
  color: var(--fg);
  box-shadow: 0 1px 2px rgb(0 0 0 / 8%);
}
</style>
