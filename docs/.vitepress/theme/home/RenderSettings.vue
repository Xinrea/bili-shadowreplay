<script setup lang="ts">
/** The app's RenderSettingsFields, in its default 按画质 mode. */
import Icon from "./Icon.vue";

withDefaults(defineProps<{ quality?: string }>(), { quality: "均衡（默认）" });
</script>

<template>
  <div class="fields">
    <span class="field">
      <span class="m-label">编码模式</span>
      <span class="m-input select">按画质<Icon name="chevron-down" :size="14" /></span>
    </span>
    <span class="field">
      <span class="m-label">画质</span>
      <span class="m-input select">{{ quality }}<Icon name="chevron-down" :size="14" /></span>
    </span>
    <p class="m-hint">根据画质档位自动设置码率。画质越高，文件通常越大。</p>
  </div>
</template>

<style scoped>
.fields {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.m-hint {
  margin: 0;
}
</style>
