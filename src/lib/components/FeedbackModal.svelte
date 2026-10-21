<script lang="ts">
  import { CircleCheck, FolderOpen, ImagePlus, LoaderCircle, Send, X } from "lucide-svelte";
  import { invoke, TAURI_ENV } from "../invoker";
  import { formatSize } from "../size-format";

  let { show = $bindable(false) }: { show?: boolean } = $props();

  // Limits of the PowerLive feedback API.
  const MAX_MESSAGE_CHARS = 5000;
  const MAX_EMAIL_BYTES = 254;
  const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
  const MAX_IMAGE_PIXELS = 10_000_000;
  const MAX_IMAGE_SIDE = 16384;

  type Screenshot = { dataUrl: string; name: string; size: number };

  // The draft survives closing the modal; it is cleared once the feedback is received.
  let message = $state("");
  let email = $state("");
  let screenshot: Screenshot | null = $state(null);
  let imageBusy = $state(false);
  let submitting = $state(false);
  let error = $state("");
  let receipt = $state("");
  let fileInput: HTMLInputElement | undefined = $state();

  // The API counts code points, not UTF-16 units.
  const messageChars = $derived(Array.from(message.trim()).length);
  const emailError = $derived(validateEmail(email.trim()));
  const canSubmit = $derived(
    !submitting && !imageBusy && messageChars <= MAX_MESSAGE_CHARS && !emailError
  );

  function close() {
    show = false;
    // A received feedback leaves nothing to come back to.
    if (receipt) {
      receipt = "";
      error = "";
    }
  }

  function validateEmail(value: string): string {
    if (!value) return "";
    if (new TextEncoder().encode(value).length > MAX_EMAIL_BYTES) return "邮箱过长";
    return /^[^\s@<>",;]+@[^\s@<>",;.][^\s@<>",;]*\.[^\s@<>",;.]+$/.test(value)
      ? ""
      : "邮箱格式不正确";
  }

  function readDataUrl(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  }

  /** A PNG or JPEG within the API limits; anything else is re-encoded as a scaled JPEG. */
  async function prepareScreenshot(file: Blob): Promise<{ dataUrl: string; size: number }> {
    const bitmap = await createImageBitmap(file).catch(() => {
      throw new Error("无法读取该图片，请选择 PNG 或 JPEG 截图");
    });
    try {
      const { width, height } = bitmap;
      if (
        (file.type === "image/png" || file.type === "image/jpeg") &&
        file.size <= MAX_IMAGE_BYTES &&
        width * height <= MAX_IMAGE_PIXELS &&
        Math.max(width, height) <= MAX_IMAGE_SIDE
      ) {
        return { dataUrl: await readDataUrl(file), size: file.size };
      }
      let scale = Math.min(
        1,
        Math.sqrt(MAX_IMAGE_PIXELS / (width * height)),
        MAX_IMAGE_SIDE / Math.max(width, height)
      );
      for (let attempt = 0; attempt < 6; attempt++) {
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.floor(width * scale));
        canvas.height = Math.max(1, Math.floor(height * scale));
        const context = canvas.getContext("2d")!;
        // JPEG has no alpha; keep transparent areas readable.
        context.fillStyle = "#fff";
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
        const blob = await new Promise<Blob | null>((resolve) =>
          canvas.toBlob(resolve, "image/jpeg", 0.9)
        );
        if (blob && blob.size <= MAX_IMAGE_BYTES) {
          return { dataUrl: await readDataUrl(blob), size: blob.size };
        }
        scale *= 0.75;
      }
      throw new Error("截图过大，请换一张图片");
    } finally {
      bitmap.close();
    }
  }

  async function attach(file: Blob, name: string) {
    imageBusy = true;
    error = "";
    try {
      screenshot = { ...(await prepareScreenshot(file)), name };
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      imageBusy = false;
    }
  }

  function handleFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (file) attach(file, file.name);
  }

  function handlePaste(event: ClipboardEvent) {
    const item = Array.from(event.clipboardData?.items ?? []).find((item) =>
      item.type.startsWith("image/")
    );
    const file = item?.getAsFile();
    if (!file) return;
    event.preventDefault();
    attach(file, "粘贴的截图");
  }

  async function submit() {
    if (!canSubmit) return;
    submitting = true;
    error = "";
    receipt = "";
    try {
      const result = await invoke<{ id: string }>("submit_feedback", {
        message,
        email: email.trim() || null,
        screenshot: screenshot?.dataUrl ?? null,
      });
      receipt = result.id;
      message = "";
      screenshot = null;
    } catch (e) {
      error = String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (show && event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      close();
    }
  }}
/>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="ui-modal-backdrop"
    onmousedown={(event) => event.target === event.currentTarget && close()}
  >
    <div
      class="ui-modal w-full max-w-lg"
      role="dialog"
      aria-modal="true"
      aria-labelledby="feedback-title"
    >
      <div class="ui-modal-header">
        <h3 id="feedback-title">反馈问题</h3>
        <button
          type="button"
          class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon -mr-1.5 text-fg-muted"
          onclick={close}
          aria-label="关闭"
        >
          <X class="h-4 w-4" />
        </button>
      </div>

      {#if receipt}
        <div class="ui-modal-body flex flex-col items-center gap-2 py-8 text-center">
          <CircleCheck class="h-10 w-10 text-success-fg" />
          <p class="text-[14px] font-semibold text-fg">反馈已收到，感谢！</p>
          <p class="text-[12px] text-fg-muted">
            编号 <span class="select-text font-mono">{receipt}</span>
          </p>
        </div>
        <div class="ui-modal-footer">
          <button type="button" class="ui-btn ui-btn-primary" onclick={close}>完成</button>
        </div>
      {:else}
        <div class="ui-modal-body space-y-4">
          <div class="space-y-1.5">
            <div class="flex items-baseline justify-between gap-4">
              <label class="ui-label" for="feedback-message">问题说明（可选）</label>
              <span
                class="text-[11px] tabular-nums"
                class:text-fg-faint={messageChars <= MAX_MESSAGE_CHARS}
                class:text-danger-fg={messageChars > MAX_MESSAGE_CHARS}
              >
                {messageChars} / {MAX_MESSAGE_CHARS}
              </span>
            </div>
            <textarea
              id="feedback-message"
              class="ui-input w-full resize-y"
              rows="6"
              bind:value={message}
              onpaste={handlePaste}
              disabled={submitting}
              placeholder="描述遇到的问题、复现步骤或建议；可以直接粘贴截图"
            ></textarea>
          </div>

          <div class="space-y-1.5">
            <label class="ui-label" for="feedback-email">联系邮箱（可选）</label>
            <input
              id="feedback-email"
              type="email"
              class="ui-input w-full"
              bind:value={email}
              disabled={submitting}
              placeholder="仅用于回复这条反馈，例如 user@example.com"
            />
            {#if emailError}
              <p class="text-[11px] text-danger-fg">{emailError}</p>
            {/if}
          </div>

          <div class="space-y-1.5">
            <span class="ui-label">截图（可选）</span>
            <input
              bind:this={fileInput}
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif,image/bmp"
              class="hidden"
              onchange={handleFile}
            />
            {#if screenshot}
              <div class="flex items-center gap-3 rounded-[8px] border border-line bg-control p-2">
                <img
                  src={screenshot.dataUrl}
                  alt="截图预览"
                  class="h-12 w-20 shrink-0 rounded-[6px] border border-line object-cover"
                />
                <div class="min-w-0 flex-1 text-[12px]">
                  <p class="truncate text-fg" title={screenshot.name}>{screenshot.name}</p>
                  <p class="text-fg-muted">{formatSize(screenshot.size)}</p>
                </div>
                <button
                  type="button"
                  class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                  disabled={submitting}
                  onclick={() => (screenshot = null)}
                  aria-label="移除截图"
                  title="移除截图"
                >
                  <X class="h-3.5 w-3.5" />
                </button>
              </div>
            {:else}
              <div class="flex items-center gap-3">
                <button
                  type="button"
                  class="ui-btn ui-btn-sm"
                  disabled={submitting || imageBusy}
                  onclick={() => fileInput?.click()}
                >
                  {#if imageBusy}
                    <LoaderCircle class="h-3.5 w-3.5 animate-spin" />处理中…
                  {:else}
                    <ImagePlus class="h-3.5 w-3.5" />选择图片
                  {/if}
                </button>
                <span class="ui-hint">支持 PNG / JPEG，过大的图片会自动压缩</span>
              </div>
            {/if}
          </div>

          <p class="ui-hint">
            提交时会自动附带最近的运行日志（已隐去账号凭据），便于排查问题。
            {#if TAURI_ENV}
              <button
                type="button"
                class="inline-flex items-center gap-1 text-accent-fg hover:underline"
                onclick={() => invoke("open_log_folder")}
              >
                <FolderOpen class="h-3 w-3" />查看日志
              </button>
            {/if}
          </p>

          {#if error}
            <p class="text-[12px] leading-relaxed text-danger-fg" role="alert">{error}</p>
          {/if}
        </div>
        <div class="ui-modal-footer">
          <button type="button" class="ui-btn ui-btn-ghost" onclick={close}>取消</button>
          <button
            type="button"
            class="ui-btn ui-btn-primary"
            disabled={!canSubmit}
            onclick={submit}
          >
            {#if submitting}
              <LoaderCircle class="h-3.5 w-3.5 animate-spin" />正在提交…
            {:else}
              <Send class="h-3.5 w-3.5" />提交反馈
            {/if}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}
