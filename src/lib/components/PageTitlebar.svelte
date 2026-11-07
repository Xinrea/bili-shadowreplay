<script lang="ts">
  import type { Snippet } from "svelte";
  import { pageTitlebarSink } from "../page-titlebar";

  // Renders nothing in place: hands the page's title and actions to the
  // window title bar, which shows them while the page is active.

  interface Props {
    title: string;
    subtitle?: Snippet;
    /** Page actions. */
    children?: Snippet;
  }

  let { title, subtitle, children }: Props = $props();

  const sink = pageTitlebarSink();

  $effect(() => {
    sink?.({ title, subtitle, actions: children });
    return () => sink?.(null);
  });
</script>
