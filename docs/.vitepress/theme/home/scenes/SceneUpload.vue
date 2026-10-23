<script setup lang="ts">
import { reactive, ref } from "vue";
import Icon from "../Icon.vue";
import MainWindow from "../MainWindow.vue";
import Pointer from "../Pointer.vue";
import Thumb from "../Thumb.vue";
import { useScene } from "../scene";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ start: [duration: number]; done: [] }>();

type Status = "draft" | "exporting" | "posting" | "posted";
type Row = {
  id: number;
  title: string;
  source: string;
  sourceTitle: string;
  scene: "dusk" | "city" | "studio" | "sea" | "forest";
  status: Status;
  bvid?: string;
  account: string;
  time: string;
};

const badges: Record<Status, { label: string; tone: string }> = {
  draft: { label: "草稿", tone: "" },
  exporting: { label: "生成视频中", tone: "accent" },
  posting: { label: "投稿中", tone: "accent" },
  posted: { label: "已投稿", tone: "success" },
};

const history = (): Row[] => [
  { id: 2, title: "这周的新番，哪部最让你上头", source: "切片", sourceTitle: "深夜杂谈集锦", scene: "studio", status: "posted", bvid: "BV1qT4y1x7aR", account: "星野的切片号", time: "2026/10/22 23:41" },
  { id: 1, title: "凌晨三点的城市夜景", source: "素材", sourceTitle: "城市夜景 选区 4", scene: "city", status: "posted", bvid: "BV1Hm4y1c7Lp", account: "星野的切片号", time: "2026/10/21 02:15" },
];

const DESC = "深渊满星全过程，弹幕和字幕都已压制。";
const TAGS = "原神,深境螺旋,星野Hoshino";

const state = reactive({
  rows: history(),
  editor: false,
  desc: "",
  tags: "",
  account: "",
  focus: "" as "" | "desc" | "tags",
});

const draft = (): Row => ({
  id: 3,
  title: "深渊满星集锦",
  source: "切片",
  sourceTitle: "深渊满星集锦",
  scene: "dusk",
  status: "draft",
  account: "",
  time: "2026/10/23 21:08",
});

const root = ref<HTMLElement | null>(null);
const el = (name: string) => root.value?.querySelector(`[data-ref="${name}"]`);

const { pointer } = useScene({
  active: () => props.active,
  root,
  reset() {
    state.rows = history();
    state.editor = false;
    state.desc = "";
    state.tags = "";
    state.account = "";
    state.focus = "";
  },
  async run(s) {
    await s.wait(600);
    state.rows = [draft(), ...history()];
    await s.wait(900);
    await s.click(el("edit"), 900);
    state.editor = true;
    await s.wait(500);
    await s.click(el("account"), 700);
    state.account = "星野的切片号";
    await s.wait(200);
    await s.click(el("desc"), 600);
    state.focus = "desc";
    await s.type(DESC, (value) => (state.desc = value), 50);
    await s.click(el("tags"), 600);
    state.focus = "tags";
    await s.type(TAGS, (value) => (state.tags = value), 50);
    state.focus = "";
    await s.wait(300);
    await s.click(el("post"), 700);
    const row = state.rows[0];
    row.account = state.account;
    row.status = "exporting";
    await s.wait(1600);
    row.status = "posting";
    await s.wait(1400);
    row.status = "posted";
    row.bvid = "BV1Zr4y1N7Sx";
    await s.wait(1300);
    await s.click(el("close"), 700);
    state.editor = false;
    await s.wait(300);
    await s.move(el("bvid"), 700);
    await s.wait(1500);
  },
  onStart: (duration) => emit("start", duration),
  onDone: () => emit("done"),
});
</script>

<template>
  <div ref="root" class="m-root m-follow scene">
    <MainWindow page="投稿">
      <template #subtitle>
        <span class="subtitle">选择素材、切片或视频文件投稿到 B 站；切片会在投稿时按时间轴生成视频。</span>
      </template>
      <template #actions>
        <span class="m-btn ghost sm icon"><Icon name="refresh-cw" :size="16" /></span>
        <span class="m-btn primary new"><Icon name="plus" :size="14" />新建投稿</span>
      </template>

      <div class="page">
        <div class="m-card filters">
          <span class="tabs">
            <span class="tab active">全部 <i class="num">{{ state.rows.length }}</i></span>
            <span class="tab">草稿 <i class="num">{{ state.rows.filter((r) => r.status === "draft").length }}</i></span>
            <span class="tab">进行中 <i class="num">{{ state.rows.filter((r) => r.status === "exporting" || r.status === "posting").length }}</i></span>
            <span class="tab">已投稿 <i class="num">{{ state.rows.filter((r) => r.status === "posted").length }}</i></span>
            <span class="tab">失败 <i class="num">0</i></span>
          </span>
          <span class="m-input search"><Icon name="search" :size="13" /><span class="placeholder">搜索标题、来源或 BV 号</span></span>
        </div>

        <div class="m-card table">
          <div class="thead">
            <span>投稿</span><span>状态</span><span>账号</span><span>更新时间</span><span>操作</span>
          </div>
          <TransitionGroup name="m-rise">
            <div v-for="row in state.rows" :key="row.id" class="tr">
              <span class="cell main">
                <span class="cover"><Thumb :scene="row.scene" /></span>
                <span class="text">
                  <b>{{ row.title }}</b>
                  <small><span class="m-badge">{{ row.source }}</span>{{ row.sourceTitle }}</small>
                </span>
              </span>
              <span class="cell status">
                <span class="m-badge" :class="badges[row.status].tone">
                  <Icon v-if="row.status === 'exporting' || row.status === 'posting'" name="loader-circle" :size="12" class="spin" />
                  {{ badges[row.status].label }}
                </span>
                <a v-if="row.bvid" class="bvid" :data-ref="row.id === 3 ? 'bvid' : undefined">{{ row.bvid }}</a>
                <small v-else-if="row.status === 'draft'">投稿时生成视频</small>
              </span>
              <span class="cell" :class="{ faint: !row.account }">{{ row.account || "未选择" }}</span>
              <span class="cell time"><Icon name="calendar" :size="13" />{{ row.time }}</span>
              <span class="cell ops">
                <span class="op" :data-ref="row.id === 3 ? 'edit' : undefined">
                  <Icon :name="row.status === 'draft' ? 'pencil' : 'eye'" :size="14" />
                </span>
                <span class="op danger"><Icon name="trash-2" :size="14" /></span>
              </span>
            </div>
          </TransitionGroup>
        </div>
      </div>
    </MainWindow>

    <Transition name="m-fade">
      <div v-if="state.editor" class="m-backdrop">
        <div class="m-modal dialog">
          <div class="m-modal-header">
            <h3>
              <Icon name="upload" :size="16" class="accent" />
              {{ state.rows[0].status === "draft" ? "编辑投稿" : "投稿详情" }}
            </h3>
            <span class="m-btn sm icon ghost"><Icon name="x" :size="16" /></span>
          </div>
          <div class="m-modal-body body">
            <div v-if="state.rows[0].status === 'posted'" class="banner success">
              <Icon name="circle-check" :size="16" />
              <span>已投稿，B 站稿件审核通过后即可观看。</span>
              <b>{{ state.rows[0].bvid }} <Icon name="external-link" :size="12" /></b>
            </div>
            <div v-else-if="state.rows[0].status !== 'draft'" class="banner accent">
              <Icon name="loader-circle" :size="16" class="spin" />
              <span>
                {{ state.rows[0].status === "exporting" ? "正在生成切片视频，完成后会自动投稿。" : "正在投稿，关闭窗口不会中断。" }}
              </span>
              <span class="m-btn sm"><Icon name="list-checks" :size="14" />查看进度</span>
            </div>

            <div class="columns">
              <div class="left">
                <section>
                  <div class="row-head">
                    <span class="m-label">视频封面</span>
                    <span class="m-btn sm">编辑封面</span>
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
                    <p class="m-hint">投稿时按切片的时间轴生成视频。</p>
                    <div class="danmu">
                      <span>
                        <b>压制弹幕</b>
                        <span class="m-hint">与切片编辑窗口的弹幕设置相同</span>
                      </span>
                      <span class="m-switch on" />
                    </div>
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
                <label>
                  <span class="m-label">标签</span>
                  <span class="m-input" :class="{ focus: state.focus === 'tags' }" data-ref="tags">
                    <span v-if="state.tags || state.focus === 'tags'">{{ state.tags }}<span v-if="state.focus === 'tags'" class="m-caret" /></span>
                    <span v-else class="placeholder">输入视频标签，用逗号分隔</span>
                  </span>
                </label>
              </div>
            </div>
          </div>
          <div class="m-modal-footer footer">
            <span class="m-hint">{{ state.rows[0].status === "draft" ? "修改会自动保存为草稿" : "" }}</span>
            <span class="buttons">
              <span class="m-btn" data-ref="close">关闭</span>
              <span v-if="state.rows[0].status === 'draft'" class="m-btn primary" data-ref="post">
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

.subtitle {
  overflow: hidden;
  font-size: 12px;
  color: var(--muted);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.new {
  height: 28px;
  gap: 6px;
  border-radius: 7px;
  padding: 0 10px;
}

.page {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px;
}

.filters {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
}

.tabs {
  display: inline-flex;
  gap: 2px;
  border-radius: 9px;
  background: var(--sunken);
  padding: 3px;
}

.tab {
  display: inline-flex;
  height: 28px;
  align-items: center;
  gap: 6px;
  border-radius: 7px;
  padding: 0 12px;
  font-size: 12px;
  color: var(--muted);
}

.tab.active {
  background: var(--selected);
  color: var(--fg);
  box-shadow: 0 1px 2px rgb(0 0 0 / 8%);
}

.tab i {
  font-style: normal;
  color: var(--faint);
}

.search {
  width: 224px;
  gap: 8px;
  color: var(--faint);
}

.table {
  overflow: hidden;
}

.thead,
.tr {
  display: grid;
  grid-template-columns: 2.4fr 1.2fr 1fr 1.3fr 0.7fr;
  align-items: center;
}

.thead {
  border-bottom: 1px solid var(--line);
  background: color-mix(in srgb, var(--control) 60%, transparent);
  font-size: 11px;
  font-weight: 500;
  color: var(--muted);
}

.thead span,
.cell {
  padding: 10px 16px;
}

.tr + .tr {
  border-top: 1px solid var(--line);
}

.cell {
  overflow: hidden;
  font-size: 12px;
  color: var(--fg-2);
  white-space: nowrap;
}

.cell.faint {
  color: var(--faint);
}

.main {
  display: flex;
  align-items: center;
  gap: 12px;
}

.cover {
  width: 64px;
  height: 40px;
  flex-shrink: 0;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 6px;
}

.text {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.text b {
  overflow: hidden;
  font-weight: 500;
  color: var(--fg);
  text-overflow: ellipsis;
}

.text small {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--muted);
}

.status {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
}

.status small {
  font-size: 11px;
  color: var(--faint);
}

.bvid {
  font-size: 11px;
  color: var(--accent-fg);
}

.time {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ops {
  display: flex;
  gap: 4px;
}

.op {
  display: inline-flex;
  width: 28px;
  height: 28px;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  color: var(--accent-fg);
}

.op.danger {
  color: var(--danger-fg);
}

.spin {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Editor dialog */
.dialog {
  width: 780px;
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
  grid-template-columns: 260px minmax(0, 1fr);
  gap: 20px;
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
  font-size: 22px;
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
  gap: 12px;
}

.fields label {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.select {
  justify-content: space-between;
}

.select :deep(svg) {
  color: var(--muted);
}

.desc {
  min-height: 62px;
}

.footer {
  justify-content: space-between;
}

.buttons {
  display: flex;
  gap: 8px;
}
</style>
