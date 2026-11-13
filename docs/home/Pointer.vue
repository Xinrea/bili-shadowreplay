<script setup lang="ts">
import type { Pointer } from "./scene";

defineProps<{ pointer: Pointer }>();
</script>

<template>
  <div
    class="pointer"
    :class="{ shown: pointer.shown, down: pointer.down }"
    :style="{
      transform: `translate(${pointer.x}px, ${pointer.y}px)`,
      transitionDuration: `${pointer.ms}ms, 200ms`,
    }"
    aria-hidden="true"
  >
    <span class="ring" />
    <svg width="22" height="22" viewBox="0 0 22 22">
      <path
        d="M4 2.5v15.2l4.1-3.9 2.7 6.1 2.6-1.1-2.7-6h5.8z"
        fill="#111"
        stroke="#fff"
        stroke-width="1.4"
        stroke-linejoin="round"
      />
    </svg>
  </div>
</template>

<style scoped>
.pointer {
  position: absolute;
  top: 0;
  left: 0;
  z-index: 50;
  margin: -3px 0 0 -4px;
  opacity: 0;
  pointer-events: none;
  transition-property: transform, opacity;
  transition-timing-function: cubic-bezier(0.45, 0.05, 0.25, 1), ease;
  filter: drop-shadow(0 2px 3px rgb(0 0 0 / 30%));
}

.pointer.shown {
  opacity: 1;
}

.pointer svg {
  position: relative;
  transition: transform 120ms ease;
}

.pointer.down svg {
  transform: scale(0.86);
}

.ring {
  position: absolute;
  top: 3px;
  left: 4px;
  width: 30px;
  height: 30px;
  margin: -15px 0 0 -15px;
  border-radius: 50%;
  background: rgb(10 132 255 / 35%);
  opacity: 0;
  transform: scale(0.4);
  transition: opacity 260ms ease, transform 260ms ease;
}

.pointer.down .ring {
  opacity: 1;
  transform: scale(1);
}
</style>
