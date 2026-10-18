<script lang="ts">
  import type { Snippet } from "svelte";
  import {
    providePageTitlebar,
    type PageTitlebarContent,
  } from "../page-titlebar";

  // One page of the main window. Pages stay mounted and are only hidden, so
  // each keeps its state; `titlebar` receives what the page's `PageTitlebar`
  // declares.

  interface Props {
    visible: boolean;
    titlebar?: PageTitlebarContent | null;
    children: Snippet;
  }

  let { visible, titlebar = $bindable(), children }: Props = $props();

  providePageTitlebar((content) => (titlebar = content));
</script>

<div class="page" class:visible>
  {@render children()}
</div>

<style>
  .page {
    display: flex;
    height: 0;
    flex-direction: column;
    overflow: hidden;
    opacity: 0;
    transform: translateX(100%);
    transition:
      opacity 0.5s ease-in-out,
      transform 0.3s ease-in-out;
  }

  .page.visible {
    height: 100%;
    opacity: 1;
    transform: translateX(0);
  }
</style>
