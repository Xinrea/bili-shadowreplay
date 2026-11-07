<script lang="ts">
  import { MoveDiagonal2, RotateCw, X } from "lucide-svelte";

  // The frame around the selected element with its delete, rotate and scale
  // handles. Positions are CSS pixels of the stage, `left`/`top` the center.

  interface Props {
    left: number;
    top: number;
    width: number;
    height: number;
    /** Degrees, clockwise. */
    rotation: number;
    onDelete: () => void;
    onRotate: (event: PointerEvent) => void;
    onScale: (event: PointerEvent) => void;
  }

  let { left, top, width, height, rotation, onDelete, onRotate, onScale }: Props = $props();
</script>

<div
  class="pointer-events-none absolute border-[1.5px] border-accent"
  style:left="{left}px"
  style:top="{top}px"
  style:width="{width}px"
  style:height="{height}px"
  style:transform="translate(-50%, -50%) rotate({rotation}deg)"
>
  <button
    type="button"
    class="handle -left-2.5 -top-2.5 hover:text-danger-fg"
    title="删除"
    aria-label="删除"
    onpointerdown={(event) => event.stopPropagation()}
    onclick={onDelete}
  >
    <X class="h-3 w-3" />
  </button>
  <button
    type="button"
    class="handle -right-2.5 -top-2.5 cursor-grab"
    title="旋转，按住 Shift 以 15° 为步长"
    aria-label="旋转"
    onpointerdown={onRotate}
  >
    <RotateCw class="h-3 w-3" />
  </button>
  <button
    type="button"
    class="handle -bottom-2.5 -left-2.5 cursor-nesw-resize"
    title="缩放"
    aria-label="缩放"
    onpointerdown={onScale}
  >
    <MoveDiagonal2 class="h-3 w-3 rotate-90" />
  </button>
  <button
    type="button"
    class="handle -bottom-2.5 -right-2.5 cursor-nwse-resize"
    title="缩放"
    aria-label="缩放"
    onpointerdown={onScale}
  >
    <MoveDiagonal2 class="h-3 w-3" />
  </button>
</div>

<style>
  .handle {
    pointer-events: auto;
    position: absolute;
    display: flex;
    height: 20px;
    width: 20px;
    align-items: center;
    justify-content: center;
    border-radius: 9999px;
    border: 1px solid rgb(var(--c-line-strong));
    background: white;
    color: #3a4252;
    box-shadow: 0 1px 4px rgb(0 0 0 / 30%);
  }
</style>
