<script setup lang="ts">
/**
 * A drawn stand-in for a stream cover or video frame, so the demo needs no
 * screenshots of real streamers.
 */
import { useId } from "vue";

withDefaults(defineProps<{ scene: "dusk" | "city" | "studio" | "sea" | "forest"; dim?: boolean }>(), {
  dim: false,
});

const id = useId();
</script>

<template>
  <svg
    class="thumb"
    :class="{ dim }"
    viewBox="0 0 320 180"
    preserveAspectRatio="xMidYMid slice"
    aria-hidden="true"
  >
    <template v-if="scene === 'dusk'">
      <defs>
        <linearGradient :id="`${id}s`" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#2b2a63" />
          <stop offset="0.55" stop-color="#c1588a" />
          <stop offset="1" stop-color="#f6a86a" />
        </linearGradient>
      </defs>
      <rect width="320" height="180" :fill="`url(#${id}s)`" />
      <circle cx="214" cy="104" r="30" fill="#ffd9a0" opacity="0.9" />
      <path d="M0 118 60 82l38 22 52-44 56 50 40-26 74 40v56H0z" fill="#5a2f6b" />
      <path d="M0 140 72 108l50 24 70-36 62 34 66-20v70H0z" fill="#3a1f4f" />
      <path d="M0 160 90 140l80 14 70-12 80 16v22H0z" fill="#1f1233" />
    </template>

    <template v-else-if="scene === 'city'">
      <defs>
        <linearGradient :id="`${id}s`" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#0b1330" />
          <stop offset="1" stop-color="#3b2361" />
        </linearGradient>
      </defs>
      <rect width="320" height="180" :fill="`url(#${id}s)`" />
      <circle cx="250" cy="40" r="14" fill="#f3e8ff" opacity="0.85" />
      <g fill="#1a1838">
        <rect x="10" y="70" width="40" height="110" />
        <rect x="56" y="40" width="34" height="140" />
        <rect x="96" y="88" width="46" height="92" />
        <rect x="150" y="56" width="30" height="124" />
        <rect x="186" y="96" width="52" height="84" />
        <rect x="244" y="64" width="36" height="116" />
        <rect x="286" y="90" width="34" height="90" />
      </g>
      <g fill="#ffcf7a" opacity="0.85">
        <rect x="62" y="52" width="5" height="5" /><rect x="74" y="64" width="5" height="5" />
        <rect x="62" y="88" width="5" height="5" /><rect x="156" y="70" width="5" height="5" />
        <rect x="166" y="96" width="5" height="5" /><rect x="250" y="80" width="5" height="5" />
        <rect x="262" y="104" width="5" height="5" /><rect x="18" y="86" width="5" height="5" />
        <rect x="104" y="100" width="5" height="5" /><rect x="196" y="110" width="5" height="5" />
      </g>
      <rect y="160" width="320" height="20" fill="#ff5fa2" opacity="0.18" />
    </template>

    <template v-else-if="scene === 'studio'">
      <defs>
        <radialGradient :id="`${id}s`" cx="0.7" cy="0.3" r="0.9">
          <stop offset="0" stop-color="#ffe2c6" />
          <stop offset="1" stop-color="#d9826b" />
        </radialGradient>
      </defs>
      <rect width="320" height="180" :fill="`url(#${id}s)`" />
      <rect x="22" y="22" width="70" height="52" rx="4" fill="#fff4e6" opacity="0.55" />
      <circle cx="166" cy="84" r="30" fill="#4b2a33" />
      <path d="M100 180c4-46 32-62 66-62s62 16 66 62z" fill="#6b3a44" />
      <rect x="226" y="96" width="10" height="56" rx="5" fill="#2d1c22" />
      <rect x="220" y="76" width="22" height="30" rx="11" fill="#3b262d" />
    </template>

    <template v-else-if="scene === 'sea'">
      <defs>
        <linearGradient :id="`${id}s`" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#8fd3ff" />
          <stop offset="0.6" stop-color="#d8f1ff" />
          <stop offset="0.6" stop-color="#2b8bd6" />
          <stop offset="1" stop-color="#11508f" />
        </linearGradient>
      </defs>
      <rect width="320" height="180" :fill="`url(#${id}s)`" />
      <circle cx="80" cy="50" r="18" fill="#fff8d6" />
      <path d="M180 108h120l-22 10H196z" fill="#f7f7f2" />
      <path d="M236 108V66l30 42z" fill="#ffffff" />
      <g stroke="#ffffff" stroke-opacity="0.35" stroke-width="2" fill="none">
        <path d="M20 132q20-6 40 0t40 0" /><path d="M150 150q20-6 40 0t40 0" />
        <path d="M60 166q20-6 40 0t40 0" />
      </g>
    </template>

    <template v-else>
      <defs>
        <linearGradient :id="`${id}s`" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#c8f0d4" />
          <stop offset="1" stop-color="#7fc79a" />
        </linearGradient>
      </defs>
      <rect width="320" height="180" :fill="`url(#${id}s)`" />
      <path d="M0 120q80-40 160-10t160-20v90H0z" fill="#4c9a6a" />
      <g fill="#2f6d4a">
        <path d="M40 130 58 80l18 50z" /><path d="M70 136 92 70l22 66z" />
        <path d="M228 120 248 66l20 54z" /><path d="M262 128 280 82l18 46z" />
      </g>
      <path d="M0 150q90-20 170 4t150-6v32H0z" fill="#2b5c40" />
    </template>
  </svg>
</template>

<style scoped>
.thumb {
  display: block;
  width: 100%;
  height: 100%;
}

.thumb.dim {
  filter: brightness(0.75) saturate(0.8);
}
</style>
