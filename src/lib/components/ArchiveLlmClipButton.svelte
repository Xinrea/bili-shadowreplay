<script module lang="ts">
  import { SvelteMap } from "svelte/reactivity";

  type LlmClipPhase = "summary" | "clipping" | "done" | "failed";
  interface LlmClipState {
    phase: LlmClipPhase;
    message: string;
  }
  /** Shared across instances so list rows and calendar menus stay in sync. */
  const llmClipStates = new SvelteMap<string, LlmClipState>();
</script>

<script lang="ts">
  import { AlertCircle, CheckCircle2, Loader2, Scissors } from "lucide-svelte";
  import { archiveKey } from "../archives";
  import type { RecordItem, RecordSummary, SummaryHighlight } from "../db";
  import {
    generateEventId,
    generateMaterials,
    type ClipRangeParams,
    type LlmConfig,
  } from "../interface";
  import { invoke } from "../invoker";
  import { announceTask, openTaskCenter, settleTask, wasCancelled } from "../stores/tasks";

  interface Props {
    archive: RecordItem;
    menu?: boolean;
    onaction?: () => void;
  }

  let { archive, menu = false, onaction }: Props = $props();

  const key = $derived(archiveKey(archive));
  const state = $derived(llmClipStates.get(key));
  const running = $derived(state?.phase === "summary" || state?.phase === "clipping");

  function llmClipTitle() {
    if (running) return state?.message || "正在生成 LLM 切片…";
    if (state?.phase === "done") return state.message;
    if (state?.phase === "failed") return `生成失败：${state.message}`;
    return "一键生成 LLM 切片（带弹幕）";
  }

  function delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  function parseSummaryHighlights(
    value: string | null | undefined,
  ): SummaryHighlight[] {
    if (!value) return [];
    try {
      const parsed = JSON.parse(value);
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }

  function llmSummaryStageText(stage: string) {
    switch (stage) {
      case "extracting_audio":
        return "正在提取完整音频…";
      case "transcribing":
        return "正在生成字幕…";
      case "summarizing":
        return "正在总结直播内容…";
      default:
        return "正在生成 Summary…";
    }
  }

  /**
   * 一键生成 LLM 切片：
   * 1. 复用 Summary 链路，没有可用 Summary 时先自动生成；
   * 2. 读取 Summary 中的高光时间段；
   * 3. 逐个调用 generateMaterials 生成带弹幕的独立素材。
   */
  async function generateLlmClips() {
    if (running) return;
    // 菜单动作（日历视图的 onaction 会关闭菜单并清空响应式 archive）触发后
    // 组件随之卸载，先截留目标和状态键的快照，整个异步流程只使用快照。
    const target = archive;
    const stateKey = key;
    const setState = (next: LlmClipState) => llmClipStates.set(stateKey, next);
    onaction?.();

    try {
      setState({ phase: "summary", message: "正在读取 Summary…" });

      let summary = await invoke<RecordSummary | null>("get_archive_summary", {
        platform: target.platform,
        roomId: target.room_id,
        liveId: target.live_id,
      });

      // 没有可复用的 Summary 时，先校验 LLM 配置，再走 Summary 链路重新生成并轮询结果
      if (!summary || summary.status !== "success") {
        const llmConfig = await invoke<LlmConfig>("get_llm_config");
        if (!(llmConfig.model || "").trim()) {
          throw new Error("请先在设置中配置 Summary 使用的 LLM 模型");
        }
        if (llmConfig.provider === "openai" && !(llmConfig.api_key || "").trim()) {
          throw new Error("请先在设置中配置 LLM API Key");
        }

        setState({ phase: "summary", message: "正在生成 Summary…" });
        await invoke("generate_archive_summary", {
          platform: target.platform,
          roomId: target.room_id,
          liveId: target.live_id,
          force: false,
        });

        let loaded = false;
        for (let attempt = 0; attempt < 3600; attempt++) {
          await delay(2000);
          summary = await invoke<RecordSummary | null>("get_archive_summary", {
            platform: target.platform,
            roomId: target.room_id,
            liveId: target.live_id,
          });
          if (!summary) continue;
          if (summary.status === "success") {
            loaded = true;
            break;
          }
          if (summary.status === "failed") {
            throw new Error(summary.error_message || "Summary 生成失败");
          }
          setState({
            phase: "summary",
            message: llmSummaryStageText(summary.stage),
          });
        }
        if (!loaded) {
          throw new Error("Summary 生成超时");
        }
      }

      const highlights = parseSummaryHighlights(summary.highlights_json).filter(
        (highlight) => highlight.end_seconds > highlight.start_seconds,
      );
      if (highlights.length === 0) {
        setState({ phase: "done", message: "Summary 中没有可用的高光时间段" });
        return;
      }

      const localOffset =
        parseInt(
          localStorage.getItem(`local_offset:${target.live_id}`) || "0",
          10,
        ) || 0;
      let succeeded = 0;
      let cancelled = 0;
      const failures: string[] = [];

      for (let index = 0; index < highlights.length; index++) {
        const highlight = highlights[index];
        setState({
          phase: "clipping",
          message: `正在生成 ${index + 1}/${highlights.length}：${highlight.title}`,
        });
        const eventId = generateEventId();
        const params: ClipRangeParams = {
          title: target.title,
          note: highlight.title,
          cover: "",
          platform: target.platform,
          room_id: target.room_id,
          live_id: target.live_id,
          ranges: [
            { start: highlight.start_seconds, end: highlight.end_seconds },
          ],
          danmu: true,
          local_offset: localOffset,
          fix_encoding: false,
        };
        announceTask({
          id: eventId,
          task_type: "generate_materials",
          metadata: JSON.stringify({
            params: { ...params, cover: "" },
            create_project: false,
            merge: false,
          }),
        });
        openTaskCenter(eventId);
        try {
          await generateMaterials(eventId, params, false, false);
          succeeded++;
        } catch (error) {
          const message = String(error);
          // 从任务中心取消，或任务页等其他入口取消（invoke 包装层会给后端错误加前缀）。
          if (wasCancelled(eventId) || message.endsWith("任务已取消")) {
            cancelled++;
          } else {
            failures.push(`${highlight.title}：${message}`);
          }
        } finally {
          settleTask(eventId);
        }
      }

      if (succeeded === 0 && failures.length === 0) {
        // 剩余任务均为用户主动取消，不算生成失败
        setState({
          phase: "done",
          message: `已取消 ${cancelled}/${highlights.length} 个切片的生成`,
        });
      } else if (succeeded === 0) {
        setState({
          phase: "failed",
          message: `全部生成失败（共 ${highlights.length} 个）：${failures.join("；")}`,
        });
        alert(`LLM 切片生成失败：\n${failures.join("\n")}`);
      } else {
        const extras: string[] = [];
        if (failures.length > 0) {
          extras.push(`${failures.length} 个失败：${failures.join("；")}`);
        }
        if (cancelled > 0) {
          extras.push(`${cancelled} 个已取消`);
        }
        setState({
          phase: "done",
          message: `已生成 ${succeeded}/${highlights.length} 个切片${
            extras.length > 0 ? `，${extras.join("，")}` : ""
          }`,
        });
        if (failures.length > 0) {
          alert(`部分切片生成失败：\n${failures.join("\n")}`);
        }
      }
    } catch (error) {
      const message = String(error);
      setState({ phase: "failed", message });
      alert(`LLM 切片生成失败：${message}`);
    }
  }
</script>

<button
  type="button"
  class={`${menu ? "ui-menu-item w-full" : "archive-action"} hover:bg-accent-soft`}
  class:text-success-fg={state?.phase === "done"}
  class:text-danger-fg={state?.phase === "failed"}
  class:text-accent-fg={state?.phase !== "done" && state?.phase !== "failed"}
  title={llmClipTitle()}
  aria-label={`一键 LLM 切片：${archive.title}`}
  disabled={running || archive.length <= 0}
  onclick={() => void generateLlmClips()}
>
  {#if running}
    <Loader2 class="h-4 w-4 animate-spin motion-reduce:animate-none" />
  {:else if state?.phase === "done"}
    <CheckCircle2 class="h-4 w-4" />
  {:else if state?.phase === "failed"}
    <AlertCircle class="h-4 w-4" />
  {:else}
    <Scissors class="h-4 w-4" />
  {/if}
  {#if menu}<span>一键 LLM 切片</span>{/if}
</button>
