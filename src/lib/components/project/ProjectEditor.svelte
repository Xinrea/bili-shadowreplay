<script lang="ts">
  import {
    BrainCircuit,
    CircleHelp,
    Download,
    Eraser,
    Film,
    Loader2,
    MessageSquareText,
    Pause,
    Play,
    Plus,
    Redo2,
    Scissors,
    Settings,
    Trash2,
    Undo2,
    Upload,
    Volume2,
    VolumeX,
    X,
  } from "lucide-svelte";
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { save } from "@tauri-apps/plugin-dialog";
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
    mergeRemoteTimeline,
    parseSrt,
    parseTimeline,
    placeClips,
    projectCues,
    removeClip,
    removeSubtitle,
    setClipSubtitles,
    splitClip,
    timelineDuration,
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
  let showStyleEditor = $state(false);
  let showShortcutHelp = $state(false);
  let showAiModal = $state(false);
  let aiOverwrite = $state(false);
  let aiRegenerate = $state(false);
  let aiRunning = $state(false);
  let exporting: string | null = $state(null);
  let requestingUpload = $state(false);
  let saveState: "saved" | "pending" | "saving" | "error" = $state("saved");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saving: Promise<void> | null = null;
  // The saved timeline as last seen, to tell the backend's edits from local ones.
  let syncedTimeline = "";
  let danmuSaved: Promise<void> = Promise.resolve();

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

  /**
   * Take a saved project, merging the backend's own edits of its timeline into
   * the one edited here: danmaku switched on the uploads page, clips of a
   * deleted material, recognized subtitles. `base` is what the saved timeline
   * was made from: the one just sent when saving, or else the last one seen.
   */
  function applyProjectView(view: ProjectItem, base?: ProjectTimeline) {
    project = view;
    if (!base && view.timeline === syncedTimeline) return;
    const from = base ?? parseTimeline(syncedTimeline);
    syncedTimeline = view.timeline;
    const merged = mergeRemoteTimeline(timeline, from, parseTimeline(view.timeline));
    if (merged !== timeline) timeline = merged;
  }

  async function loadProject() {
    try {
      const view = await invoke<ProjectItem>("get_project", { id: projectId });
      // Together, so a refresh during the loads below merges into this timeline.
      let loaded = parseTimeline(view.timeline);
      if (!loaded.subtitle_style) loaded = { ...loaded, subtitle_style: loadSubtitleStyle(view.room_id) };
      timeline = loaded;
      syncedTimeline = view.timeline;
      project = view;
      title = view.title;
      await set_title(view.title || "切片");
      await loadMaterials();
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

  // Keep the project current, merging the backend's edits into local ones.
  const projectRefresh = createStateRefresh(async () => {
    if (!project) return;
    // A save in flight brings back its own result; read what it left.
    while (saving) await saving;
    applyProjectView(await invoke<ProjectItem>("get_project", { id: projectId }));
  });
  // Deleted materials leave the library, and their clips the timeline.
  const materialRefresh = createStateRefresh(loadMaterials);

  onMount(() => {
    void loadProject();
    return watchStateChanges(projectRefresh, ["projects-changed"]);
  });

  onMount(() => watchStateChanges(materialRefresh, ["videos-changed"]));

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
    // Tauri destroys the window once this settles: finish the edit in progress
    // (closing does not blur it) and wait until everything is saved.
    unlistenClose = await current.onCloseRequested(async () => {
      endEdit();
      await saveAll();
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

  /** Save the timeline now, after any save already running, until it is saved. */
  async function flushSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    while (saving) await saving;
    if (saveState === "saved") return;
    const snapshot = timeline;
    saveState = "saving";
    let saved = false;
    let editedMeanwhile = false;
    saving = (async () => {
      try {
        const view = await invoke<ProjectItem>("update_project_timeline", {
          id: projectId,
          timeline: snapshot,
          base: parseTimeline(syncedTimeline),
        });
        editedMeanwhile = timeline !== snapshot;
        // Merged against what was sent: the backend keeps its danmaku setting
        // and drops clips of materials deleted meanwhile.
        applyProjectView(view, snapshot);
        saved = true;
        saveState = editedMeanwhile ? "pending" : "saved";
      } catch (error) {
        log.error("Failed to save project timeline:", error);
        saveState = "error";
      }
    })().finally(() => (saving = null));
    await saving;
    // Edits made during the save.
    if (saved && editedMeanwhile) await flushSave();
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
    // Danmaku is a setting, not an edit, so history leaves it as it is.
    timeline = { ...previous, danmu: timeline.danmu };
    scheduleSave();
  }

  function redo() {
    const next = redoStack[redoStack.length - 1];
    if (!next) return;
    undoStack = [...undoStack, timeline];
    redoStack = redoStack.slice(0, -1);
    timeline = { ...next, danmu: timeline.danmu };
    scheduleSave();
  }

  /** Switch the project's danmaku setting, which has a command of its own. */
  async function setDanmu(danmu: boolean) {
    timeline = { ...timeline, danmu };
    // Saved in order; exports and uploads wait for it, as they read the saved value.
    const request = danmuSaved.then(() => invoke("update_project_danmu", { id: projectId, danmu }));
    danmuSaved = request.then(
      () => {},
      () => {},
    );
    try {
      await request;
    } catch (error) {
      timeline = { ...timeline, danmu: !danmu };
      alert("修改弹幕设置失败：" + error);
    }
  }

  /** Save the timeline, the title and the danmaku switch. */
  async function saveAll() {
    await Promise.all([flushSave(), saveInfo(), danmuSaved]);
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
    // Bring along subtitles already recognized for the material.
    let srt = "";
    try {
      srt = await invoke<string>("get_video_subtitle", { id: materialId });
    } catch (error) {
      console.warn("Failed to load material subtitles:", error);
    }
    // Insert into the timeline as it is after the wait, which other edits may have changed.
    const at = Math.min(index, timeline.clips.length);
    let next = insertClips(timeline, [{ material_id: materialId, start: 0, end: info.duration }], at);
    const added = next.clips.length > timeline.clips.length ? next.clips[at] : undefined;
    if (srt && added) next = setClipSubtitles(next, added.id, parseSrt(srt));
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

  async function saveInfo() {
    if (!project || title === project.title) return;
    try {
      // Only the title: the note is edited from the projects list.
      const saved = title;
      await invoke("update_project_info", { id: projectId, title: saved });
      // So a later change back is saved too, before the refresh reports it.
      if (project) project = { ...project, title: saved };
      await set_title(saved || "切片");
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
    const overwrite = aiOverwrite;
    const regenerate = aiRegenerate;
    const wanted = (clipId: string) =>
      overwrite || !timeline.subtitles.some((subtitle) => subtitle.clip_id === clipId);
    const cuesByMaterial = new Map<number, ReturnType<typeof parseSrt>>();
    try {
      for (const materialId of new Set(timeline.clips.map((clip) => clip.material_id))) {
        if (!timeline.clips.some((clip) => clip.material_id === materialId && wanted(clip.id))) continue;
        let srt = regenerate ? "" : await invoke<string>("get_video_subtitle", { id: materialId });
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
        cuesByMaterial.set(materialId, parseSrt(srt));
      }
      // Recognizing can take minutes of other edits: fill the clips as they
      // are now, as one undo step, keeping subtitles typed in the meantime.
      let next = timeline;
      for (const clip of timeline.clips) {
        const cues = cuesByMaterial.get(clip.material_id);
        if (!cues || !wanted(clip.id)) continue;
        next = setClipSubtitles(
          next,
          clip.id,
          cues.filter((cue) => cue.end > clip.start && cue.start < clip.end),
        );
      }
      commit(next);
    } catch (error) {
      alert("生成字幕失败：" + error);
    } finally {
      aiRunning = false;
    }
  }

  // ---- Export --------------------------------------------------------------

  /** Write the timeline to a file the user picks, or download it on the web. */
  async function exportProject() {
    if (exporting || !project) return;
    if (timeline.clips.length === 0) {
      alert("时间轴上还没有片段");
      return;
    }
    const fileName = `${(title || "切片").replace(/[\\/:*?"<>|]/g, "_")}.mp4`;
    let target: string | null = null;
    if (TAURI_ENV) {
      target = await save({
        title: "导出切片",
        defaultPath: fileName,
        filters: [{ name: "MP4 视频", extensions: ["mp4"] }],
      });
      if (!target) return;
    }
    // The export renders the saved danmaku setting, and the subtitles below
    // must include any the backend has added meanwhile.
    await saveAll();
    await projectRefresh.run();
    const eventId = generateEventId();
    exporting = eventId;
    let unlisten: (() => void) | null = null;
    // The export may finish before the invoke returns where it is written.
    let started: ExportStarted | null = null;
    let finished: { success: boolean; message: string } | null = null;
    const settle = () => {
      if (!started || !finished) return;
      exporting = null;
      if (finished.success) {
        void exportFinished(started, fileName);
      } else if (!wasCancelled(eventId) && !finished.message.endsWith("已取消")) {
        alert(finished.message);
      }
    };
    try {
      unlisten = await listen<{ success: boolean; message: string }>(`progress-finished:${eventId}`, (event) => {
        unlisten?.();
        finished = event.payload;
        settle();
      });
      announceTask({
        id: eventId,
        task_type: "export_project",
        metadata: JSON.stringify({ project_id: projectId, title }),
      });
      started = await invoke<ExportStarted>("export_project", {
        eventId,
        id: projectId,
        timeline,
        subtitleAss: subtitles.length > 0 ? subtitleAss : null,
        target,
        base: parseTimeline(syncedTimeline),
      });
      openTaskCenter(eventId);
      settle();
    } catch (error) {
      unlisten?.();
      exporting = null;
      alert("导出失败：" + error);
    } finally {
      settleTask(eventId);
    }
  }

  async function exportFinished(started: ExportStarted, fileName: string) {
    if (started.download) {
      const a = document.createElement("a");
      a.href = await get_static_url("cache", started.download);
      a.download = fileName;
      a.click();
      return;
    }
    // Tauri's dialog plugin makes confirm() asynchronous.
    if (await confirm(`导出完成：${started.path}\n\n是否在文件夹中显示？`)) {
      void invoke("show_in_folder", { path: started.path });
    }
  }

  /** Continue in the main window, which starts a draft for this project. */
  async function requestUpload() {
    if (requestingUpload) return;
    requestingUpload = true;
    try {
      // The draft takes its title from the saved project.
      await saveAll();
      await invoke("request_upload", { source: "project", id: projectId });
    } catch (error) {
      alert("打开投稿失败：" + error);
    } finally {
      requestingUpload = false;
    }
  }

  // ---- Keyboard ------------------------------------------------------------

  function handleKeydown(event: KeyboardEvent) {
    if (showAiModal && event.key === "Escape") {
      event.preventDefault();
      showAiModal = false;
      return;
    }
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
        KeyS: () => void saveAll(),
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
      <span class="mx-1 h-4 w-px bg-[#2b3340]"></span>
      <button
        type="button"
        class="titlebar-toggle"
        aria-pressed={timeline.danmu}
        title="导出和投稿生成视频时{timeline.danmu ? '压制' : '不压制'}弹幕（片段范围内共 {visibleDanmuCount} 条）"
        onclick={() => void setDanmu(!timeline.danmu)}
      >
        <MessageSquareText class="h-3.5 w-3.5" />
        弹幕{timeline.danmu ? "：开" : "：关"}
      </button>
      <button
        type="button"
        class="titlebar-button"
        title="在主窗口为这个切片新建投稿"
        disabled={requestingUpload || timeline.clips.length === 0}
        onclick={requestUpload}
      >
        <Upload class="h-3.5 w-3.5" />
        投稿
      </button>
      <button
        type="button"
        class="titlebar-button primary"
        title={TAURI_ENV ? "导出为视频文件" : "导出并下载视频文件"}
        disabled={exporting !== null || timeline.clips.length === 0}
        onclick={exportProject}
      >
        {#if exporting}
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
          导出中
        {:else}
          <Download class="h-3.5 w-3.5" />
          导出
        {/if}
      </button>
    {/snippet}
  </TitleBar>

  {#if loadError}
    <div class="flex flex-1 items-center justify-center text-sm text-red-300">加载切片失败：{loadError}</div>
  {/if}

  <div class="flex min-h-0 flex-1 flex-col" class:hidden={!!loadError}>
    <div class="relative flex min-h-0 flex-1 overflow-hidden">
      <MaterialBin
        materials={binMaterials}
        {roomId}
        onAdd={(material) => void addMaterial(material.id)}
        onPreview={(material) => void invoke("open_clip", { videoId: material.id })}
      />

      <div class="relative min-w-0 flex-1 bg-black">
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

      <InspectorPanel
        title="字幕"
        tabs={[{ id: "subtitle", label: "字幕", count: subtitles.length }]}
        activeTab="subtitle"
        collapsed={false}
        docked
      >
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
                  oninput={(event) => {
                    // One undo step per edit, but saved while typing.
                    timeline = updateSubtitleText(timeline, entry.subtitle.id, event.currentTarget.value);
                    scheduleSave();
                  }}
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
      </InspectorPanel>
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
</div>

{#if showAiModal}
  <div class="ui-modal-backdrop z-[1100]">
    <div class="ui-modal w-[440px] max-w-full" role="dialog" aria-modal="true" aria-labelledby="ai-subtitle-title">
      <div class="ui-modal-header">
        <h3 id="ai-subtitle-title" class="flex items-center gap-2">
          <BrainCircuit class="h-4 w-4 shrink-0 text-accent-fg" />
          AI 字幕
        </h3>
        <button
          type="button"
          class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5"
          aria-label="关闭"
          onclick={() => (showAiModal = false)}
        >
          <X class="h-4 w-4" />
        </button>
      </div>
      <div class="ui-modal-body space-y-4">
        <p class="text-[13px] leading-relaxed text-fg-secondary">
          按素材识别字幕，每个素材只识别一次，结果会缓存并按片段范围放入时间轴。
        </p>
        <div class="space-y-2.5">
          <label class="flex cursor-pointer items-start gap-2.5">
            <input type="checkbox" bind:checked={aiOverwrite} class="ui-checkbox mt-0.5" />
            <span class="text-[13px] text-fg-secondary">
              覆盖已有字幕的片段
              <span class="block text-[11px] text-fg-muted">默认只为还没有字幕的片段识别</span>
            </span>
          </label>
          <label class="flex cursor-pointer items-start gap-2.5">
            <input type="checkbox" bind:checked={aiRegenerate} class="ui-checkbox mt-0.5" />
            <span class="text-[13px] text-fg-secondary">
              忽略缓存，重新识别
              <span class="block text-[11px] text-fg-muted">识别结果不准确，或更换了字幕生成器时使用</span>
            </span>
          </label>
        </div>
      </div>
      <div class="ui-modal-footer">
        <button type="button" class="ui-btn" onclick={() => (showAiModal = false)}>取消</button>
        <button type="button" class="ui-btn ui-btn-primary" onclick={generateSubtitles}>开始识别</button>
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

  .titlebar-button,
  .titlebar-toggle {
    display: inline-flex;
    height: 28px;
    align-items: center;
    gap: 6px;
    border: 1px solid #344052;
    border-radius: 7px;
    background: #202733;
    padding: 0 11px;
    color: #dce5f3;
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    transition: background-color 150ms ease, border-color 150ms ease, color 150ms ease;
  }

  .titlebar-button:hover:not(:disabled),
  .titlebar-toggle:hover {
    border-color: #4a5a70;
  }

  .titlebar-button.primary {
    border-color: transparent;
    background: #0a84ff;
    color: white;
  }

  .titlebar-button.primary:hover:not(:disabled) {
    background: rgb(10 132 255 / 88%);
  }

  .titlebar-button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .titlebar-toggle {
    color: #8c98aa;
  }

  .titlebar-toggle[aria-pressed="true"] {
    border-color: rgb(10 132 255 / 70%);
    background: rgb(10 132 255 / 15%);
    color: #55b9ff;
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
</style>
