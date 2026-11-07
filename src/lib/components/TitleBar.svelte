<script lang="ts">
  import type { Snippet } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    CUSTOM_WINDOW_CONTROLS,
    MAC_TRAFFIC_LIGHTS,
    TRAFFIC_LIGHTS_INSET,
    windowDrag,
    windowState,
  } from "../window-chrome";

  // The top bar of every app window. It is the drag region, keeps clear of the
  // macOS traffic lights and draws the window buttons elsewhere. Outside the
  // desktop app it is a plain toolbar.

  interface Props {
    title?: string;
    /** "auto" follows the window theme; preview windows are always "dark". */
    tone?: "auto" | "dark";
    /** Leave room for the macOS traffic lights at the left edge. */
    trafficLightInset?: boolean;
    /**
     * Distance of the bar from the window's left edge, so the inset only
     * covers the part of the traffic lights that still overlaps it.
     */
    trafficLightOffset?: number;
    /** Left side, before the title. */
    leading?: Snippet;
    /** Left side, after the title. */
    children?: Snippet;
    /** Right side, before the window buttons. */
    actions?: Snippet;
  }

  let {
    title = "",
    tone = "auto",
    trafficLightInset = true,
    trafficLightOffset = 0,
    leading,
    children,
    actions,
  }: Props = $props();

  let maximized = $derived($windowState.maximized);
  let insetPadding = $derived(
    trafficLightInset && MAC_TRAFFIC_LIGHTS && !$windowState.fullscreen
      ? TRAFFIC_LIGHTS_INSET - trafficLightOffset
      : 0,
  );
</script>

<header
  class="titlebar {tone}"
  class:has-window-controls={CUSTOM_WINDOW_CONTROLS}
  style:padding-left={insetPadding > 16 ? `${insetPadding}px` : undefined}
  use:windowDrag
>
  <div class="titlebar-main">
    {@render leading?.()}
    {#if title}
      <h1 class="titlebar-title" {title}>{title}</h1>
    {/if}
    {@render children?.()}
  </div>

  {#if actions}
    <div class="titlebar-actions">{@render actions()}</div>
  {/if}

  {#if CUSTOM_WINDOW_CONTROLS}
    <div class="window-controls">
      <button
        type="button"
        title="最小化"
        aria-label="最小化"
        onclick={() => getCurrentWindow().minimize()}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" /></svg>
      </button>
      <button
        type="button"
        title={maximized ? "向下还原" : "最大化"}
        aria-label={maximized ? "向下还原" : "最大化"}
        onclick={() => getCurrentWindow().toggleMaximize()}
      >
        {#if maximized}
          <svg viewBox="0 0 10 10" aria-hidden="true">
            <path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" />
          </svg>
        {:else}
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5h9v9h-9z" /></svg>
        {/if}
      </button>
      <button
        type="button"
        class="close"
        title="关闭"
        aria-label="关闭"
        onclick={() => getCurrentWindow().close()}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" /></svg>
      </button>
    </div>
  {/if}
</header>

<style>
  .titlebar {
    position: relative;
    display: flex;
    height: 40px;
    flex: 0 0 40px;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 16px;
    border-bottom: 1px solid;
    /* Matches the main window's sidebar collapsing beside it. */
    transition: padding-left 200ms cubic-bezier(0.4, 0, 0.2, 1);
  }

  .titlebar.has-window-controls {
    padding-right: 0;
  }

  .titlebar.auto {
    border-color: rgb(var(--c-line));
    background: rgb(var(--c-chrome));
    color: rgb(var(--c-fg));
  }

  .titlebar.dark {
    border-color: #2b3340;
    background: #141922;
    color: #e9eef8;
  }

  .titlebar-main {
    display: flex;
    min-width: 0;
    height: 100%;
    flex: 1;
    align-items: center;
    gap: 10px;
  }

  .titlebar-title {
    overflow: hidden;
    margin: 0;
    flex-shrink: 1;
    font-size: 14px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .titlebar-actions {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: 6px;
  }

  /* Icon buttons passed in by pages and windows. */
  .titlebar :global(.titlebar-icon-button) {
    display: inline-flex;
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    transition:
      background-color 120ms ease,
      color 120ms ease;
  }

  .auto :global(.titlebar-icon-button) {
    color: rgb(var(--c-fg-muted));
  }

  .auto :global(.titlebar-icon-button:hover:not(:disabled)),
  .auto :global(.titlebar-icon-button.active) {
    background: rgb(var(--c-control-hover));
    color: rgb(var(--c-fg));
  }

  .dark :global(.titlebar-icon-button) {
    color: #8c98aa;
  }

  .dark :global(.titlebar-icon-button:hover:not(:disabled)),
  .dark :global(.titlebar-icon-button.active) {
    background: #222a36;
    color: #dce5f3;
  }

  .titlebar :global(.titlebar-icon-button.danger:hover:not(:disabled)) {
    background: rgb(239 68 68 / 14%);
    color: #f87171;
  }

  .titlebar :global(.titlebar-icon-button:disabled) {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .titlebar :global(.titlebar-icon-button:focus-visible) {
    outline: 2px solid #0a84ff;
    outline-offset: 1px;
  }

  .window-controls {
    display: flex;
    height: 100%;
    flex-shrink: 0;
    align-self: stretch;
  }

  .window-controls button {
    display: inline-flex;
    width: 46px;
    height: 100%;
    align-items: center;
    justify-content: center;
    color: inherit;
    transition: background-color 120ms ease;
  }

  .window-controls svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }

  .window-controls .close svg {
    shape-rendering: geometricPrecision;
  }

  .auto .window-controls button:hover {
    background: rgb(var(--c-control-hover));
  }

  .dark .window-controls button:hover {
    background: rgb(255 255 255 / 8%);
  }

  .window-controls button.close:hover {
    background: #c42b1c;
    color: white;
  }

  .window-controls button:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: -2px;
  }
</style>
