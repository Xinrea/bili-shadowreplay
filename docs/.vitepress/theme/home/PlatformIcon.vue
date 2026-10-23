<script setup lang="ts">
import { platformImg, platformSvg } from "./platforms";

withDefaults(defineProps<{ id: string; size?: number }>(), { size: 16 });
</script>

<template>
  <span
    class="platform-icon"
    :class="id"
    :style="{ width: `${size}px`, height: `${size}px` }"
  >
    <img v-if="platformImg[id]" :src="platformImg[id]" alt="" />
    <span v-else class="glyph" v-html="platformSvg[id]" />
  </span>
</template>

<style scoped>
.platform-icon {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
}

.platform-icon img,
.glyph,
.glyph :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.bilibili {
  color: #fb7299;
}

.twitch {
  color: #9146ff;
}
</style>
