<script lang="ts">
  import {
    BrainCircuit,
    CircleHelp,
    Eraser,
    Film,
    Loader2,
    PanelRightClose,
    PanelRightOpen,
    Pause,
    Play,
    Plus,
    Redo2,
    Scissors,
    Settings,
    Trash2,
    Undo2,
    Volume2,
    VolumeX,
  } from "lucide-svelte";
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { get_static_url, invoke, listen, log, set_title, TAURI_ENV } from "../../invoker";
  import {
    generateEventId,
    type ExportStarted,
    type MaterialPeaks,
    type ProjectItem,
    type ProjectTimeline,
    type SubtitleStyle,
    type VideoItem,
  } from "../../interface";
  import {
    addSubtitle,
    clipAt,
    insertClips,
    parseSrt,
    parseTimeline,
    placeClips,
    projectCues,
    removeClip,
    removeSubtitle,
    setClipSubtitles,
    splitClip,
    timelineDuration,
    trimClip,
    updateSubtitleText,
    visibleSubtitles,
    MIN_SUBTITLE_DURATION,
    moveSubtitle,
  } from "../../project-timeline";
  import { ProjectPlayer, type PlayerSegment } from "../../project-player";
  import { buildSubtitleAss, loadSubtitleStyle } from "../../subtitle-ass";
  import { createStateRefresh, watchStateChanges } from "../../state-refresh";
  import { announceTask, openTaskCenter, settleTask, wasCancelled } from "../../stores/tasks";
  import AssSubtitlePreview from "../AssSubtitlePreview.svelte";
  import InspectorPanel from "../InspectorPanel.svelte";
  import ShortcutHelpModal from "../ShortcutHelpModal.svelte";
  import SubtitleStyleEditor from "../SubtitleStyleEditor.svelte";
  import TaskCenterButton from "../TaskCenterButton.svelte";
  import TitleBar from "../TitleBar.svelte";
  import MaterialBin, { type BinMaterial } from "./MaterialBin.svelte";
  import ProjectTimelineView, { type MaterialInfo } from "./ProjectTimeline.svelte";
  import ProjectUploadPanel from "./ProjectUploadPanel.svelte";
  import "./project-editor.css";

  interface Props {
    projectId: number;
  }

  let { projectId }: Props = $props();

  const HISTORY_LIMIT = 100;
  const SAVE_DELAY_MS = 800;

  let project: ProjectItem | null = $state(null);
  let loadError = $state("");
  let title = $state("");
  let note = $state("");
  let timeline: ProjectTimeline = $state.raw(parseTimeline(""));
  let undoStack: ProjectTimeline[] = $state.raw([]);
  let redoStack: ProjectTimeline[] = $state.raw([]);
  let materials: Map<number, MaterialInfo> = $state.raw(new Map());
  let binMaterials: BinMaterial[] = $state.raw([]);
  let peaks: Map<number, MaterialPeaks> = $state.raw(new Map());
  let danmuCounts: Map<number, number[]> = $state.raw(new Map());

  let time = $state(0);
  let playing = $state(false);
  let scrubbing = false;
  let zoom = $state(1);
  let volume = $state(1);
  let muted = $state(false);
  let selectedClipId: string | null = $state(null);
  let selectedSubtitleId: string | null = $state(null);
  let activeTab: "subtitle" | "clips" | "export" | "upload" = $state("subtitle");
  let inspectorCollapsed = $state(false);
  let showStyleEditor = $state(false);
  let showShortcutHelp = $state(false);
  let showAiModal = $state(false);
  let aiOverwrite = $state(false);
  let aiRegenerate = $state(false);
  let aiRunning = $state(false);
  let exporting: string | null = $state(null);
  let saveState: "saved" | "pending" | "saving" | "error" = $state("saved");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  let videoA: HTMLVideoElement = $state();
  let videoB: HTMLVideoElement = $state();
  let player: ProjectPlayer | null = $state(null);
  let activeVideo = $state(0);
  let previewContainer: HTMLElement = $state();
  let previewFrameWidth = $state(0);
  let frame = 0;
  let stopRange: (() => void) | null = null;
  let editBefore: ProjectTimeline | null = null;
  let unlistenClose: (() => void) | null = null;

  let roomId = $derived(project?.room_id ?? "");
  let duration = $derived(timelineDuration(timeline));
  let placed = $derived(placeClips(timeline));
  let subtitles = $derived(visibleSubtitles(timeline));
  let hiddenSubtitleCount = $derived(timeline.subtitles.length - subtitles.length);
  let subtitleStyle: SubtitleStyle = $derived(
    (timeline.subtitle_style as SubtitleStyle | null) ?? loadSubtitleStyle(roomId),
  );
  let subtitleAss = $derived(buildSubtitleAss(projectCues(timeline), subtitleStyle));
  let selectedClip = $derived(placed.find((item) => item.clip.id === selectedClipId) ?? null);
  let currentClip = $derived(clipAt(timeline, time));
  let coverVideoUrl = $derived(
    (currentClip && materials.get(currentClip.clip.material_id)?.url) ?? "",
  );
  let exportState = $derived(project?.export_state ?? "none");
  let visibleDanmuCount = $derived(
    placed.reduce((total, item) => {
      const times = danmuCounts.get(item.clip.material_id) ?? [];
      return total + times.filter((ts) => ts >= item.clip.start * 1000 && ts < item.clip.end * 1000).length;
    }, 0),
  );
  let segments: PlayerSegment[] = $derived(
    placed
      .filter((item) => materials.has(item.clip.material_id))
      .map((item) => ({
        clipId: item.clip.id,
        url: materials.get(item.clip.material_id).url,
        start: item.clip.start,
        end: item.clip.end,
        offset: item.offset,
      })),
  );
  // Reload the preview only when the picture changes, not for subtitle edits.
  let segmentKey = $derived(segments.map((s) => `${s.clipId}:${s.url}:${s.start}:${s.end}`).join("|"));

  // ---- Loading -------------------------------------------------------------

  async function materialInfo(video: VideoItem): Promise<MaterialInfo> {
    return {
      id: video.id,
      name: video.title || video.file,
      url: await get_static_url("output", video.file),
      coverUrl: video.cover ? await get_static_url("output", video.cover) : "",
      duration: video.duration || video.length,
    };
  }

  async function loadMaterials() {
    const list = await invoke<VideoItem[]>("get_all_videos");
    const infos = await Promise.all(list.map(materialInfo));
    materials = new Map(infos.map((info) => [info.id, info]));
    binMaterials = list.map((video, index) => ({
      id: video.id,
      name: infos[index].name,
      note: video.note,
      roomId: video.room_id,
      coverUrl: infos[index].coverUrl,
      duration: infos[index].duration,
      createdAt: video.created_at,
    }));
  }

  function applyProjectView(view: ProjectItem) {
    project = view;
  }

  async function loadProject() {
    try {
      const view = await invoke<ProjectItem>("get_project", { id: projectId });
      project = view;
      title = view.title;
      note = view.note;
      await set_title(view.title || "切片");
      await loadMaterials();
      let loaded = parseTimeline(view.timeline);
      if (!loaded.subtitle_style) loaded = { ...loaded, subtitle_style: loadSubtitleStyle(view.room_id) };
      timeline = loaded;
      void loadMaterialData();
    } catch (error) {
      loadError = String(error);
    }
  }

  /** Waveforms and danmaku times of the materials on the timeline. */
  async function loadMaterialData() {
    for (const id of new Set(timeline.clips.map((clip) => clip.material_id))) {
      if (!peaks.has(id)) {
        invoke<MaterialPeaks>("get_material_peaks", { id })
          .then((data) => (peaks = new Map(peaks).set(id, data)))
          .catch((error) => console.warn("Failed to load waveform:", error));
      }
      if (!danmuCounts.has(id)) {
        invoke<{ ts: number; type: string }[]>("get_material_danmu", { id })
          .then((entries) => {
            const times = entries.filter((entry) => entry.type === "danmu").map((entry) => entry.ts);
            danmuCounts = new Map(danmuCounts).set(id, times);
          })
          .catch((error) => console.warn("Failed to load danmaku:", error));
      }
    }
  }

  // Keep export and upload state current; never replace local edits.
  const projectRefresh = createStateRefresh(async () => {
    if (!project) return;
    applyProjectView(await invoke<ProjectItem>("get_project", { id: projectId }));
  });

  onMount(() => {
    void loadProject();
    return watchStateChanges(projectRefresh, ["projects-changed"]);
  });

  onMount(() => {
    player = new ProjectPlayer(videoA, videoB);
    const update = () => {
      if (!scrubbing) time = player.currentTime;
    };
    player.addEventListener("timeupdate", update);
    player.addEventListener("seeked", update);
    player.addEventListener("activechange", () => (activeVideo = player.activeElement === videoA ? 0 : 1));
    player.addEventListener("play", () => {
      playing = true;
      const follow = () => {
        time = player.currentTime;
        if (playing) frame = requestAnimationFrame(follow);
      };
      frame = requestAnimationFrame(follow);
    });
    const stop = () => {
      playing = false;
      cancelAnimationFrame(frame);
      time = player.currentTime;
    };
    player.addEventListener("pause", stop);
    player.addEventListener("ended", stop);
  });

  onMount(async () => {
    if (!TAURI_ENV) return;
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const current = getCurrentWebviewWindow();
    unlistenClose = await current.onCloseRequested(async (event) => {
      if (saveState === "saved") return;
      event.preventDefault();
      await flushSave();
      await current.destroy();
    });
  });

  onDestroy(() => {
    cancelAnimationFrame(frame);
    stopRange?.();
    player?.dispose();
    unlistenClose?.();
    if (saveTimer) void flushSave();
  });

  $effect(() => {
    segmentKey;
    if (player) untrack(() => player.setSegments(segments));
  });

  $effect(() => {
    if (player) {
      player.volume = volume;
      player.muted = muted;
    }
  });

  // The encoder scales and pads to 1920×1080; preview the same frame.
  $effect(() => {
    const container = previewContainer;
    if (!container) return;
    const resize = () => {
      previewFrameWidth = Math.min(container.clientWidth, (container.clientHeight * 16) / 9);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(container);
    resize();
    return () => observer.disconnect();
  });

  // ---- Editing and history -------------------------------------------------

  function scheduleSave() {
    saveState = "pending";
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void flushSave(), SAVE_DELAY_MS);
  }

  async function flushSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    if (saveState === "saved" || saveState === "saving") return;
    const snapshot = timeline;
    saveState = "saving";
    try {
      applyProjectView(await invoke<ProjectItem>("update_project_timeline", { id: projectId, timeline: snapshot }));
      saveState = timeline === snapshot ? "saved" : "pending";
      if (saveState === "pending") scheduleSave();
    } catch (error) {
      log.error("Failed to save project timeline:", error);
      saveState = "error";
    }
  }

  /** Apply an edit as one undo step. */
  function commit(next: ProjectTimeline, before: ProjectTimeline = timeline) {
    if (next === before) {
      timeline = before;
      return;
    }
    undoStack = [...undoStack.slice(-(HISTORY_LIMIT - 1)), before];
    redoStack = [];
    timeline = next;
    scheduleSave();
    void loadMaterialData();
  }

  function undo() {
    const previous = undoStack[undoStack.length - 1];
    if (!previous) return;
    redoStack = [...redoStack, timeline];
    undoStack = undoStack.slice(0, -1);
    timeline = previous;
    scheduleSave();
  }

  function redo() {
    const next = redoStack[redoStack.length - 1];
    if (!next) return;
    undoStack = [...undoStack, timeline];
    redoStack = redoStack.slice(0, -1);
    timeline = next;
    scheduleSave();
  }

  /** Begin an edit made of several updates, such as typing a subtitle. */
  function beginEdit() {
    editBefore ??= timeline;
  }

  function endEdit() {
    if (editBefore) commit(timeline, editBefore);
    editBefore = null;
  }

  async function addMaterial(materialId: number, index = timeline.clips.length) {
    let info = materials.get(materialId);
    if (!info) return;
    if (!info.duration) {
      // Old materials learn their exact length when first used.
      const video = await invoke<VideoItem>("get_video", { id: materialId });
      info = { ...info, duration: video.duration || video.length };
      materials = new Map(materials).set(materialId, info);
    }
    let next = insertClips(timeline, [{ material_id: materialId, start: 0, end: info.duration }], index);
    // Bring along subtitles already recognized for the material.
    const added = next.clips[Math.min(index, next.clips.length - 1)];
    try {
      const srt = await invoke<string>("get_video_subtitle", { id: materialId });
      if (srt && added) next = setClipSubtitles(next, added.id, parseSrt(srt));
    } catch (error) {
      console.warn("Failed to load material subtitles:", error);
    }
    commit(next);
    selectedClipId = added?.id ?? null;
  }

  function splitAtPlayhead() {
    const result = splitClip(timeline, time);
    if (!result.clipId) return;
    commit(result.timeline);
    selectedClipId = result.clipId;
  }

  function deleteSelection() {
    if (selectedSubtitleId) {
      commit(removeSubtitle(timeline, selectedSubtitleId));
      selectedSubtitleId = null;
    } else if (selectedClipId) {
      commit(removeClip(timeline, selectedClipId));
      selectedClipId = null;
    }
  }

  function setClipEdge(clipId: string, edge: "start" | "end", value: number) {
    const clip = timeline.clips.find((item) => item.id === clipId);
    if (!clip || !Number.isFinite(value)) return;
    commit(trimClip(timeline, clipId, edge, value, materials.get(clip.material_id)?.duration ?? 0));
  }

  async function saveInfo() {
    if (!project || (title === project.title && note === project.note)) return;
    try {
      await invoke("update_project_info", { id: projectId, title, note });
      await set_title(title || "切片");
    } catch (error) {
      alert("保存切片信息失败：" + error);
    }
  }

  // ---- Playback ------------------------------------------------------------

  function seek(target: number) {
    stopRange?.();
    time = Math.max(0, Math.min(target, duration));
    player?.seek(time);
  }

  function togglePlay() {
    stopRange?.();
    player?.toggle();
  }

  /** Play `start..end` of the project and stop there. */
  async function playRange(start: number, end: number) {
    if (!player) return;
    stopRange?.();
    player.pause();
    player.seek(start);
    time = start;
    let stopped = false;
    let check = 0;
    const stop = () => {
      stopped = true;
      cancelAnimationFrame(check);
      if (stopRange === stop) stopRange = null;
    };
    stopRange = stop;
    await player.play();
    const watch = () => {
      if (stopped) return;
      if (!playing) return stop();
      if (player.currentTime >= end) {
        player.pause();
        return stop();
      }
      check = requestAnimationFrame(watch);
    };
    check = requestAnimationFrame(watch);
  }

  // ---- Subtitles -----------------------------------------------------------

  function selectSubtitle(id: string, seekTo = false) {
    selectedSubtitleId = id;
    selectedClipId = null;
    const item = subtitles.find((entry) => entry.subtitle.id === id);
    if (seekTo && item) seek(item.start);
    tick().then(() =>
      document.querySelector(`[data-subtitle-row="${id}"]`)?.scrollIntoView({ block: "nearest", behavior: "smooth" }),
    );
  }

  function addSubtitleAtPlayhead() {
    const clip = clipAt(timeline, time);
    if (!clip) return;
    const end = Math.min(time + 2, clip.offset + clip.duration);
    if (end - time < MIN_SUBTITLE_DURATION) return;
    const result = addSubtitle(timeline, time, end);
    if (!result.id) return;
    commit(result.timeline);
    selectSubtitle(result.id);
    tick().then(() =>
      document.querySelector<HTMLInputElement>(`[data-subtitle-row="${result.id}"] input`)?.focus(),
    );
  }

  function insertSubtitleAfter(id: string) {
    const item = subtitles.find((entry) => entry.subtitle.id === id);
    if (!item) return;
    const start = item.end;
    const limit = item.clip.offset + item.clip.duration;
    const next = subtitles.find((entry) => entry.start >= start - 0.001 && entry.subtitle.id !== id);
    const end = Math.min(start + Math.min(5, Math.max(1, item.end - item.start)), next?.start ?? limit, duration);
    if (end - start < MIN_SUBTITLE_DURATION) {
      alert("后面没有空间添加字幕");
      return;
    }
    const result = addSubtitle(timeline, start, end);
    if (!result.id) return;
    commit(result.timeline);
    seek(start);
    selectSubtitle(result.id);
    tick().then(() =>
      document.querySelector<HTMLInputElement>(`[data-subtitle-row="${result.id}"] input`)?.focus(),
    );
  }

  function selectAdjacentSubtitle(direction: -1 | 1) {
    const index = subtitles.findIndex((entry) => entry.subtitle.id === selectedSubtitleId);
    let target: number;
    if (index >= 0) {
      target = index + direction;
    } else if (direction > 0) {
      target = subtitles.findIndex((entry) => entry.start > time + 0.001);
    } else {
      target = subtitles.length - 1;
      while (target >= 0 && subtitles[target].start >= time - 0.001) target--;
    }
    const entry = subtitles[target];
    if (entry) selectSubtitle(entry.subtitle.id, true);
  }

  /** The subtitle shortcuts act on: the selected one, else the one under the playhead. */
  function targetSubtitle() {
    const selected = subtitles.find((entry) => entry.subtitle.id === selectedSubtitleId);
    if (selected) return selected;
    const current = subtitles.find((entry) => time >= entry.start && time < entry.end);
    if (current) selectedSubtitleId = current.subtitle.id;
    return current ?? null;
  }

  /** S / Q / W / E / D: the whole subtitle, 500 ms before / after it, or its first / last 500 ms. */
  function playSubtitlePart(part: "whole" | "before" | "after" | "begin" | "end") {
    const entry = targetSubtitle();
    if (!entry) return;
    const span = 0.5;
    const { start, end } = entry;
    const ranges = {
      whole: [start, end],
      before: [start - span, start],
      after: [end, end + span],
      begin: [start, Math.min(start + span, end)],
      end: [Math.max(end - span, start), end],
    };
    const [from, to] = ranges[part];
    if (to - Math.max(0, from) > 0.01) void playRange(Math.max(0, from), Math.min(to, duration));
  }

  function setSubtitleBoundary(edge: "start" | "end") {
    const entry = targetSubtitle();
    if (!entry) return;
    const start = edge === "start" ? time : entry.start;
    const end = edge === "end" ? time : entry.end;
    if (end - start < MIN_SUBTITLE_DURATION) return;
    commit(moveSubtitle(timeline, entry.subtitle.id, start, end));
  }

  async function clearSubtitles() {
    if (timeline.subtitles.length === 0) return;
    // Tauri's dialog plugin makes confirm() asynchronous.
    if (!(await confirm("确定清空切片中的所有字幕吗？可以撤销。"))) return;
    commit({ ...timeline, subtitles: [] });
    selectedSubtitleId = null;
  }

  /** Recognize each material once and give its clips the cues in range. */
  async function generateSubtitles() {
    showAiModal = false;
    if (aiRunning) return;
    aiRunning = true;
    const before = timeline;
    let next = timeline;
    try {
      for (const materialId of new Set(timeline.clips.map((clip) => clip.material_id))) {
        const clips = next.clips.filter((clip) => clip.material_id === materialId);
        const pending = clips.filter(
          (clip) => aiOverwrite || !next.subtitles.some((subtitle) => subtitle.clip_id === clip.id),
        );
        if (pending.length === 0) continue;
        let srt = aiRegenerate ? "" : await invoke<string>("get_video_subtitle", { id: materialId });
        if (!srt) {
          const eventId = generateEventId();
          announceTask({
            id: eventId,
            task_type: "generate_video_subtitle",
            metadata: JSON.stringify({ video_id: materialId }),
          });
          openTaskCenter(eventId);
          try {
            srt = await invoke<string>("generate_video_subtitle", { eventId, id: materialId });
          } finally {
            settleTask(eventId);
          }
        }
        const cues = parseSrt(srt);
        for (const clip of pending) {
          next = setClipSubtitles(
            next,
            clip.id,
            cues.filter((cue) => cue.end > clip.start && cue.start < clip.end),
          );
        }
      }
      // Other edits may have happened while recognizing; keep only subtitles from them.
      commit({ ...timeline, subtitles: next.subtitles }, before);
    } catch (error) {
      alert("生成字幕失败：" + error);
    } finally {
      aiRunning = false;
    }
  }

  // ---- Export --------------------------------------------------------------

  async function exportProject() {
    if (exporting || !project) return;
    if (timeline.clips.length === 0) {
      alert("时间轴上还没有片段");
      return;
    }
    await flushSave();
    const eventId = generateEventId();
    exporting = eventId;
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<{ success: boolean; message: string }>(`progress-finished:${eventId}`, (event) => {
        unlisten?.();
        exporting = null;
        void projectRefresh.run();
        if (!event.payload.success && !wasCancelled(eventId) && !event.payload.message.endsWith("已取消")) {
          alert(event.payload.message);
        }
      });
      announceTask({
        id: eventId,
        task_type: "export_project",
        metadata: JSON.stringify({ project_id: projectId, title: project.title }),
      });
      const started = await invoke<ExportStarted>("export_project", {
        eventId,
        id: projectId,
        timeline,
        subtitleAss: subtitles.length > 0 ? subtitleAss : null,
      });
      if (started.task) {
        openTaskCenter(eventId);
      } else {
        unlisten();
        exporting = null;
        alert(started.message);
      }
      await projectRefresh.run();
    } catch (error) {
      unlisten?.();
      exporting = null;
      alert("导出失败：" + error);
    } finally {
      settleTask(eventId);
    }
  }

  async function downloadExport() {
    if (!project?.output) return;
    const a = document.createElement("a");
    a.href = await get_static_url("output", project.output);
    a.download = `${project.title || "切片"}.mp4`;
    a.click();
  }

  // ---- Keyboard ------------------------------------------------------------

  function handleKeydown(event: KeyboardEvent) {
    if (showStyleEditor || showShortcutHelp || showAiModal) return;
    const target = event.target instanceof Element ? event.target : null;
    if (
      target?.closest("input, textarea, select, [contenteditable='true'], [data-hotkey-block]") ||
      event.altKey
    ) {
      return;
    }
    if (event.repeat && !event.code.startsWith("Arrow")) return;
    const command = event.ctrlKey || event.metaKey;
    if (command) {
      const handled = {
        KeyZ: () => (event.shiftKey ? redo() : undo()),
        KeyY: redo,
        KeyB: splitAtPlayhead,
        KeyS: () => void flushSave(),
        Digit3: () => setSubtitleBoundary("start"),
        Digit4: () => setSubtitleBoundary("end"),
      }[event.code];
      if (handled) {
        event.preventDefault();
        handled();
      }
      return;
    }
    if (event.shiftKey && !event.code.startsWith("Arrow")) return;
    const step = event.shiftKey ? 1 / 30 : 5;
    const handled = {
      Space: togglePlay,
      ArrowLeft: () => seek(time - step),
      ArrowRight: () => seek(time + step),
      Delete: deleteSelection,
      Backspace: deleteSelection,
      KeyZ: () => selectAdjacentSubtitle(-1),
      KeyX: () => selectAdjacentSubtitle(1),
      KeyS: () => playSubtitlePart("whole"),
      KeyQ: () => playSubtitlePart("before"),
      KeyW: () => playSubtitlePart("after"),
      KeyE: () => playSubtitlePart("begin"),
      KeyD: () => playSubtitlePart("end"),
    }[event.code];
    if (handled) {
      event.preventDefault();
      handled();
    }
  }

  function formatTime(seconds: number) {
    const total = Math.max(0, seconds);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = (total % 60).toFixed(1).padStart(4, "0");
    return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${secs}` : `${minutes}:${secs}`;
  }

  const exportDescriptions = {
    material: "切片只包含一个完整素材且没有字幕和弹幕，投稿时直接使用素材文件，无需导出。",
    none: "切片还没有导出。导出会按时间轴重新编码，并压制字幕和弹幕。",
    stale: "切片在上次导出后有修改，需要重新导出。",
    ready: "导出文件与当前时间轴一致，可以直接投稿。",
  };
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="project-editor fixed inset-0 z-[1000] flex flex-col bg-[#1c1c1e] text-white">
  <TitleBar tone="dark">
    <input
      class="title-input"
      bind:value={title}
      placeholder="未命名切片"
      title="切片标题"
      onblur={saveInfo}
      onkeydown={(event) => {
        if (event.key === "Enter") (event.currentTarget as HTMLInputElement).blur();
      }}
    />
    <span class="save-state" class:error={saveState === "error"}>
      {#if saveState === "saved"}
        已保存
      {:else if saveState === "error"}
        保存失败
        <button type="button" onclick={() => void flushSave()}>重试</button>
      {:else}
        保存中…
      {/if}
    </span>
    {#snippet actions()}
      <button class="titlebar-icon-button" title="撤销 (⌘Z)" aria-label="撤销" disabled={undoStack.length === 0} onclick={undo}>
        <Undo2 class="h-4 w-4" />
      </button>
      <button class="titlebar-icon-button" title="重做 (⇧⌘Z)" aria-label="重做" disabled={redoStack.length === 0} onclick={redo}>
        <Redo2 class="h-4 w-4" />
      </button>
      <button class="titlebar-icon-button" title="快捷键说明" aria-label="快捷键说明" onclick={() => (showShortcutHelp = true)}>
        <CircleHelp class="h-4 w-4" />
      </button>
      <TaskCenterButton tone="dark" />
      <button
        class="titlebar-icon-button"
        class:active={!inspectorCollapsed}
        title={inspectorCollapsed ? "展开面板" : "收起面板"}
        aria-label={inspectorCollapsed ? "展开面板" : "收起面板"}
        aria-pressed={!inspectorCollapsed}
        onclick={() => (inspectorCollapsed = !inspectorCollapsed)}
      >
        {#if inspectorCollapsed}
          <PanelRightOpen class="h-4 w-4" />
        {:else}
          <PanelRightClose class="h-4 w-4" />
        {/if}
      </button>
    {/snippet}
  </TitleBar>

  {#if loadError}
    <div class="flex flex-1 items-center justify-center text-sm text-red-300">加载切片失败：{loadError}</div>
  {/if}

  <div class="relative flex min-h-0 flex-1 overflow-hidden" class:hidden={!!loadError}>
    <MaterialBin
      materials={binMaterials}
      {roomId}
      onAdd={(material) => void addMaterial(material.id)}
      onPreview={(material) => void invoke("open_clip", { videoId: material.id })}
    />

    <div class="flex min-w-0 flex-1 flex-col">
      <div class="relative flex-1 bg-black">
        <div bind:this={previewContainer} class="absolute inset-0 flex items-center justify-center">
          <div class="relative aspect-video bg-black" style:width="{previewFrameWidth}px">
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={videoA} class="preview-video" class:visible={activeVideo === 0} onclick={togglePlay}></video>
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={videoB} class="preview-video" class:visible={activeVideo === 1} onclick={togglePlay}></video>
            {#if player && subtitles.length > 0}
              <AssSubtitlePreview assContent={subtitleAss} video={player} fontName={subtitleStyle.fontName} />
            {/if}
            {#if timeline.clips.length === 0 && project}
              <div class="empty-preview">
                <Film class="h-8 w-8 text-gray-600" />
                <span>从左侧素材库添加素材开始剪辑</span>
              </div>
            {/if}
          </div>
        </div>
      </div>

      <div class="transport">
        <div class="flex items-center gap-1">
          <button class="icon-button" title="在播放头处分割 (⌘B)" disabled={!clipAt(timeline, time)} onclick={splitAtPlayhead}>
            <Scissors class="h-4 w-4" />
            分割
          </button>
          <button class="icon-button" title="删除选中 (Delete)" disabled={!selectedClipId && !selectedSubtitleId} onclick={deleteSelection}>
            <Trash2 class="h-4 w-4" />
            删除
          </button>
          <button class="icon-button" title="在播放头处添加字幕" disabled={!clipAt(timeline, time)} onclick={addSubtitleAtPlayhead}>
            <Plus class="h-4 w-4" />
            字幕
          </button>
        </div>
        <div class="flex items-center gap-3">
          <button class="play-button" title="播放/暂停 (空格)" onclick={togglePlay} disabled={duration === 0}>
            {#if playing}
              <Pause class="h-4 w-4" />
            {:else}
              <Play class="h-4 w-4" />
            {/if}
          </button>
          <span class="clock">{formatTime(time)} / {formatTime(duration)}</span>
        </div>
        <div class="flex items-center justify-end gap-2">
          <span class="text-[11px] text-gray-400">缩放</span>
          <input type="range" min="1" max="60" step="0.5" bind:value={zoom} class="w-28" data-hotkey-block />
          <button class="icon-button" title={muted ? "取消静音" : "静音"} onclick={() => (muted = !muted)}>
            {#if muted || volume === 0}
              <VolumeX class="h-4 w-4" />
            {:else}
              <Volume2 class="h-4 w-4" />
            {/if}
          </button>
          <input type="range" min="0" max="1" step="0.05" bind:value={volume} class="w-20" data-hotkey-block />
        </div>
      </div>

      <ProjectTimelineView
        {timeline}
        {materials}
        {peaks}
        {time}
        {zoom}
        {selectedClipId}
        {selectedSubtitleId}
        onSeek={seek}
        onScrub={(active) => (scrubbing = active)}
        onPreview={(next) => (timeline = next)}
        onCommit={(next, before) => commit(next, before)}
        onSelectClip={(id) => (selectedClipId = id)}
        onSelectSubtitle={(id) => {
          if (id) selectSubtitle(id);
          else selectedSubtitleId = null;
        }}
        onDropMaterial={(id, index) => void addMaterial(id, index)}
        onZoom={(value) => (zoom = value)}
      />
    </div>

    <InspectorPanel
      title="切片编辑"
      tabs={[
        { id: "subtitle", label: "字幕", count: subtitles.length },
        { id: "clips", label: "片段", count: timeline.clips.length },
        { id: "export", label: "导出" },
        { id: "upload", label: "投稿" },
      ]}
      bind:activeTab
      collapsed={inspectorCollapsed}
      summary={{ label: "总时长", value: formatTime(duration) }}
    >
      {#snippet actions()}
        <button
          type="button"
          class="footer-button primary"
          disabled={exporting !== null || timeline.clips.length === 0}
          onclick={exportProject}
        >
          {#if exporting}
            <Loader2 class="h-3.5 w-3.5 animate-spin" />
            导出中
          {:else}
            <Film class="h-3.5 w-3.5" />
            导出
          {/if}
        </button>
      {/snippet}

      {#if activeTab === "subtitle"}
        <div class="panel-toolbar">
          <button type="button" class="tool-button" onclick={() => (showStyleEditor = true)}>
            <Settings class="h-3.5 w-3.5" />
            样式
          </button>
          <button type="button" class="tool-button" onclick={clearSubtitles} disabled={timeline.subtitles.length === 0}>
            <Eraser class="h-3.5 w-3.5" />
            清空
          </button>
          <button
            type="button"
            class="tool-button"
            disabled={aiRunning || timeline.clips.length === 0}
            onclick={() => {
              aiOverwrite = false;
              aiRegenerate = false;
              showAiModal = true;
            }}
          >
            {#if aiRunning}
              <Loader2 class="h-3.5 w-3.5 animate-spin" />
              识别中
            {:else}
              <BrainCircuit class="h-3.5 w-3.5" />
              AI 字幕
            {/if}
          </button>
        </div>

        {#if subtitles.length > 0}
          <div class="row-list">
            {#each subtitles as entry, index (entry.subtitle.id)}
              <div
                class="row"
                data-subtitle-row={entry.subtitle.id}
                class:playing={time >= entry.start && time < entry.end}
                class:selected={selectedSubtitleId === entry.subtitle.id}
              >
                <button type="button" class="row-index" title="选中并跳转" onclick={() => selectSubtitle(entry.subtitle.id, true)}>
                  {index + 1}
                </button>
                <input
                  type="text"
                  class="row-text"
                  value={entry.subtitle.text}
                  placeholder="输入字幕文本"
                  onfocus={() => {
                    selectSubtitle(entry.subtitle.id);
                    beginEdit();
                  }}
                  oninput={(event) => (timeline = updateSubtitleText(timeline, entry.subtitle.id, event.currentTarget.value))}
                  onblur={endEdit}
                  onkeydown={(event) => {
                    if (event.isComposing || event.keyCode === 229) return;
                    if (event.key === "Enter" || event.key === "Escape") {
                      event.preventDefault();
                      event.currentTarget.blur();
                    }
                  }}
                />
                <button type="button" class="row-action" title="在后面添加字幕" onclick={() => insertSubtitleAfter(entry.subtitle.id)}>
                  <Plus class="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  class="row-action danger"
                  title="删除字幕"
                  onclick={() => commit(removeSubtitle(timeline, entry.subtitle.id))}
                >
                  <Trash2 class="h-3.5 w-3.5" />
                </button>
              </div>
            {/each}
          </div>
        {:else}
          <div class="empty-state">
            <strong>还没有字幕</strong>
            <span>点击「AI 字幕」识别，或在时间轴播放头处添加</span>
          </div>
        {/if}
        {#if hiddenSubtitleCount > 0}
          <p class="hint mt-2">另有 {hiddenSubtitleCount} 条字幕位于片段裁剪范围之外，已隐藏；拉长片段后会重新显示。</p>
        {/if}
      {:else if activeTab === "clips"}
        <div class="flex flex-col gap-3">
          <div class="field">
            <label for="project-note">备注</label>
            <textarea id="project-note" class="panel-input h-16 resize-none" bind:value={note} onblur={saveInfo} placeholder="为切片添加备注"></textarea>
          </div>

          {#if selectedClip}
            {@const material = materials.get(selectedClip.clip.material_id)}
            <div class="clip-detail">
              <div class="flex items-center justify-between gap-2">
                <span class="truncate text-[12px] font-medium" title={material?.name}>{material?.name ?? "素材已删除"}</span>
                <button type="button" class="link-button" onclick={() => void invoke("open_clip", { videoId: selectedClip.clip.material_id })}>
                  预览素材
                </button>
              </div>
              <div class="grid grid-cols-2 gap-2">
                <div class="field">
                  <label for="clip-start">入点（秒）</label>
                  <input
                    id="clip-start"
                    type="number"
                    step="0.1"
                    min="0"
                    class="panel-input"
                    value={selectedClip.clip.start.toFixed(2)}
                    onchange={(event) => setClipEdge(selectedClip.clip.id, "start", Number(event.currentTarget.value))}
                  />
                </div>
                <div class="field">
                  <label for="clip-end">出点（秒）</label>
                  <input
                    id="clip-end"
                    type="number"
                    step="0.1"
                    min="0"
                    class="panel-input"
                    value={selectedClip.clip.end.toFixed(2)}
                    onchange={(event) => setClipEdge(selectedClip.clip.id, "end", Number(event.currentTarget.value))}
                  />
                </div>
              </div>
              <p class="hint">
                片段时长 {formatTime(selectedClip.duration)}，素材总长 {formatTime(material?.duration ?? 0)}；也可以拖动时间轴上片段的左右边缘调整。
              </p>
            </div>
          {/if}

          {#if placed.length > 0}
            <div class="row-list">
              {#each placed as item (item.clip.id)}
                <div class="row" class:selected={selectedClipId === item.clip.id} class:playing={currentClip?.clip.id === item.clip.id}>
                  <button
                    type="button"
                    class="row-index"
                    onclick={() => {
                      selectedClipId = item.clip.id;
                      selectedSubtitleId = null;
                      seek(item.offset);
                    }}
                  >
                    {item.index + 1}
                  </button>
                  <button
                    type="button"
                    class="row-text truncate text-left"
                    onclick={() => {
                      selectedClipId = item.clip.id;
                      selectedSubtitleId = null;
                      seek(item.offset);
                    }}
                  >
                    {materials.get(item.clip.material_id)?.name ?? "素材已删除"}
                  </button>
                  <span class="shrink-0 font-mono text-[10px] text-gray-500">{formatTime(item.duration)}</span>
                  <button type="button" class="row-action danger" title="删除片段" onclick={() => commit(removeClip(timeline, item.clip.id))}>
                    <Trash2 class="h-3.5 w-3.5" />
                  </button>
                </div>
              {/each}
            </div>
          {:else}
            <div class="empty-state">
              <strong>时间轴是空的</strong>
              <span>从左侧素材库拖入素材，或点击素材右侧的 ＋</span>
            </div>
          {/if}
          <p class="hint">片段在主轨道上首尾相接播放：拖动片段可调整顺序，⌘B 在播放头处分割。</p>
        </div>
      {:else if activeTab === "export"}
        <div class="flex flex-col gap-4">
          <label class="toggle">
            <input
              type="checkbox"
              checked={timeline.danmu}
              onchange={(event) => commit({ ...timeline, danmu: event.currentTarget.checked })}
            />
            <span>
              压制弹幕
              <span class="hint block">片段范围内共有 {visibleDanmuCount} 条弹幕（来自素材保存的弹幕）</span>
            </span>
          </label>

          <div class="export-state {exportState}">
            {exportDescriptions[exportState]}
          </div>

          <button
            type="button"
            class="footer-button primary justify-center"
            disabled={exporting !== null || timeline.clips.length === 0}
            onclick={exportProject}
          >
            {#if exporting}
              <Loader2 class="h-3.5 w-3.5 animate-spin" />
              导出中，可在任务中心查看进度
            {:else}
              <Film class="h-3.5 w-3.5" />
              {exportState === "ready" ? "重新导出" : "导出切片"}
            {/if}
          </button>

          {#if exportState === "ready" && project?.output}
            <div class="flex flex-col gap-1">
              <span class="hint">导出文件：{project.output}</span>
              {#if !TAURI_ENV}
                <button type="button" class="link-button self-start" onclick={downloadExport}>下载导出文件</button>
              {/if}
            </div>
          {/if}

          <p class="hint">
            导出会把各片段统一缩放到 1920×1080 后拼接，并压制当前字幕（与预览一致）。任务在后台队列中执行，可以继续编辑或关闭窗口。
          </p>
        </div>
      {:else if activeTab === "upload" && project}
        <ProjectUploadPanel
          {project}
          {exportState}
          {coverVideoUrl}
          exporting={exporting !== null}
          onExport={exportProject}
          onChanged={() => void projectRefresh.run()}
        />
      {/if}
    </InspectorPanel>
  </div>
</div>

{#if showAiModal}
  <div class="fixed inset-0 z-[1100] flex items-center justify-center bg-black/50">
    <div class="w-[440px] max-w-[90vw] rounded-lg bg-[#2c2c2e] shadow-xl">
      <div class="border-b border-gray-800/50 px-4 py-3 text-sm font-medium text-gray-200">AI 字幕</div>
      <div class="space-y-3 p-4 text-[13px] text-gray-300">
        <p>按素材识别字幕，每个素材只识别一次，结果会缓存并按片段范围放入时间轴。</p>
        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={aiOverwrite} />
          覆盖已有字幕的片段
        </label>
        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={aiRegenerate} />
          忽略缓存，重新识别
        </label>
      </div>
      <div class="flex justify-end gap-2 border-t border-gray-800/50 px-4 py-3">
        <button class="footer-button" onclick={() => (showAiModal = false)}>取消</button>
        <button class="footer-button primary" onclick={generateSubtitles}>开始</button>
      </div>
    </div>
  </div>
{/if}

<SubtitleStyleEditor
  bind:show={showStyleEditor}
  {roomId}
  initialStyle={subtitleStyle}
  onSave={(style) => commit({ ...timeline, subtitle_style: style })}
  onClose={() => (showStyleEditor = false)}
/>

<ShortcutHelpModal bind:show={showShortcutHelp} />

<style>
  .title-input {
    min-width: 120px;
    max-width: 360px;
    height: 26px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    padding: 0 8px;
    color: #e5e7eb;
    font-size: 13px;
    font-weight: 500;
  }

  .title-input:hover {
    border-color: #2b3340;
  }

  .title-input:focus {
    border-color: #0a84ff;
    background: #0f131a;
    outline: none;
    box-shadow: none;
  }

  .save-state {
    color: #6b7280;
    font-size: 11px;
    white-space: nowrap;
  }

  .save-state.error {
    color: #f87171;
  }

  .save-state button {
    margin-left: 4px;
    color: #55b9ff;
  }

  .preview-video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    visibility: hidden;
    cursor: pointer;
  }

  .preview-video.visible {
    visibility: visible;
  }

  .empty-preview {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: #6b7280;
    font-size: 13px;
  }

  .transport {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    border-top: 1px solid rgb(31 41 55 / 50%);
    background: #1c1c1e;
    padding: 6px 12px;
  }

  .play-button {
    display: inline-flex;
    border-radius: 8px;
    background: #2c2c2e;
    padding: 6px;
  }

  .play-button:hover:not(:disabled) {
    background: #3c3c3e;
  }

  .clock {
    color: #d1d5db;
    font-family: ui-monospace, monospace;
    font-size: 12px;
  }

  .panel-toolbar {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    margin: 0 -16px 10px;
    background: #171c25;
    padding: 0 16px 10px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    color: #9aa6b8;
    font-size: 10px;
  }

  .clip-detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid #293442;
    border-radius: 10px;
    background: #202733;
    padding: 10px;
  }

  .link-button {
    color: #55b9ff;
    font-size: 10px;
    font-weight: 500;
    white-space: nowrap;
  }

  .toggle {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: #dce5f3;
    font-size: 12px;
    cursor: pointer;
  }

  .toggle input {
    margin-top: 2px;
  }

  .export-state {
    border: 1px solid #293442;
    border-radius: 8px;
    background: #202733;
    padding: 10px;
    color: #c4cfdd;
    font-size: 11px;
    line-height: 1.6;
  }

  .export-state.ready {
    border-color: rgb(48 209 88 / 35%);
    color: #a7f3d0;
  }

  .export-state.stale {
    border-color: rgb(255 159 10 / 35%);
    color: #fcd9a6;
  }
</style>
