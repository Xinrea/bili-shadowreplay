<script setup lang="ts">
import { h } from "vue";
import { icons } from "./icons";

const props = withDefaults(defineProps<{ name: string; size?: number; stroke?: number }>(), {
  size: 16,
  stroke: 2,
});

const Nodes = () => (icons[props.name] ?? []).map(([tag, attrs]) => h(tag, attrs));
</script>

<template>
  <svg
    xmlns="http://www.w3.org/2000/svg"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="stroke"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    class="icon"
  >
    <Nodes />
  </svg>
</template>

<style scoped>
.icon {
  flex-shrink: 0;
}
</style>
