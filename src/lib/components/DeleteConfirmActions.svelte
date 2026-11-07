<script lang="ts">
  import { tick } from "svelte";
  import { Loader2 } from "lucide-svelte";

  interface Props {
    onconfirm: () => Promise<void>;
    oncancel: () => void;
    busy?: boolean;
    confirmLabel?: string;
    align?: "center" | "end";
  }

  let {
    onconfirm,
    oncancel,
    busy = $bindable(false),
    confirmLabel = "删除",
    align = "center",
  }: Props = $props();
  let error = $state("");

  async function confirm() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      // Flush the pending state before starting the potentially slow operation.
      await tick();
      await onconfirm();
    } catch (cause) {
      error = `${confirmLabel}失败：${cause instanceof Error ? cause.message : String(cause)}`;
    } finally {
      busy = false;
    }
  }

  function cancel() {
    if (!busy) oncancel();
  }
</script>

<div class="w-full space-y-3">
  {#if busy}
    <p role="status" class="text-center text-[12px] text-fg-secondary">
      正在处理中，请稍候。文件较大时可能需要更长时间。
    </p>
  {/if}
  {#if error}
    <p role="alert" class="break-words text-[12px] text-danger-fg">{error}</p>
  {/if}
  <div class="flex gap-2" class:justify-center={align === "center"} class:justify-end={align === "end"}>
    <button class="ui-btn min-w-[72px]" disabled={busy} onclick={cancel}>取消</button>
    <button class="ui-btn ui-btn-danger min-w-[72px]" disabled={busy} aria-busy={busy} onclick={confirm}>
      {#if busy}
        <Loader2 size={14} class="animate-spin" aria-hidden="true" />
        正在处理中…
      {:else}
        {confirmLabel}
      {/if}
    </button>
  </div>
</div>
