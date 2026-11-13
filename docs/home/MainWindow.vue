<script setup lang="ts">
/** The main window's frame: sidebar, title bar and page area. */
import Icon from "./Icon.vue";

defineProps<{ page: string }>();

const pages = [
  { label: "总览", icon: "layout-dashboard" },
  { label: "直播间", icon: "video" },
  { label: "录播", icon: "history" },
  { label: "素材", icon: "film" },
  { label: "切片", icon: "clapperboard" },
  { label: "投稿", icon: "upload" },
  { label: "任务", icon: "list" },
  { label: "助手", icon: "brain" },
  { label: "账号", icon: "users" },
  { label: "设置", icon: "settings" },
  { label: "关于", icon: "info" },
];
</script>

<template>
  <div class="main-window">
    <aside class="sidebar">
      <div class="sidebar-head">
        <Icon name="panel-left" :size="16" />
      </div>
      <nav class="sidebar-nav">
        <div
          v-for="item in pages"
          :key="item.label"
          class="nav-item"
          :class="{ active: item.label === page }"
        >
          <Icon :name="item.icon" :size="16" />
          <span>{{ item.label }}</span>
        </div>
        <div class="nav-item palette">
          <Icon name="command" :size="16" />
          <span>命令面板</span>
          <kbd>⇧⌘P</kbd>
        </div>
      </nav>
    </aside>
    <div class="column">
      <header class="m-titlebar">
        <span class="title">{{ page }}</span>
        <slot name="subtitle" />
        <div class="actions">
          <slot name="actions" />
          <span class="divider" />
          <span class="m-btn ghost sm icon"><Icon name="list-checks" :size="16" /></span>
        </div>
      </header>
      <div class="page">
        <slot />
      </div>
    </div>
  </div>
</template>

<style scoped>
.main-window {
  display: flex;
  width: 100%;
  height: 100%;
}

.sidebar {
  display: flex;
  width: 192px;
  flex-shrink: 0;
  flex-direction: column;
  background: var(--chrome);
}

.sidebar-head {
  display: flex;
  height: 40px;
  align-items: center;
  padding-left: 82px;
  color: var(--muted);
}

.sidebar-nav {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 2px;
  border-right: 1px solid var(--line);
  padding: 4px 8px 12px;
}

.nav-item {
  display: flex;
  height: 32px;
  align-items: center;
  gap: 10px;
  border-radius: 8px;
  padding: 0 10px;
  font-size: 13px;
  color: var(--fg-2);
}

.nav-item.active {
  background: var(--accent-soft);
  color: var(--accent-fg);
  font-weight: 500;
}

.nav-item.palette {
  margin-top: auto;
  color: var(--muted);
}

.nav-item kbd {
  margin-left: auto;
  font-family: inherit;
  font-size: 10px;
  color: var(--faint);
}

.column {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
}

.m-titlebar {
  background: var(--canvas);
}

.actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

.divider {
  width: 1px;
  height: 16px;
  margin: 0 2px;
  background: var(--line);
}

.page {
  position: relative;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}
</style>
