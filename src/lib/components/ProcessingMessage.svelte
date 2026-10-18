<script lang="ts">
  import { Bot } from "lucide-svelte";
</script>

<div class="flex justify-start">
  <div class="flex max-w-2xl items-start gap-2.5">
    <div
      class="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full bg-accent text-white"
    >
      <Bot class="h-3.5 w-3.5" />
    </div>

    <div class="flex flex-col space-y-1">
      <span class="flex h-6 items-center text-[12px] font-medium text-fg-secondary">
        小轴
      </span>

      <div
        class="rounded-[10px] border border-line bg-surface px-3.5 py-2.5"
      >
        <div class="flex items-center gap-2">
          <div
            class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-accent border-t-transparent"
          ></div>
          <span class="text-[13px] text-fg-muted"
            >正在思考...</span
          >
        </div>
      </div>
    </div>
  </div>
</div>
