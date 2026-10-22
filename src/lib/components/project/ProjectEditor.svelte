<script lang="ts">
  import {
    ArrowDown,
    ArrowUp,
    BrainCircuit,
    CircleHelp,
    Copy,
    Download,
    Eraser,
    Film,
    Loader2,
    MessageSquareText,
    Music,
    Pause,
    Play,
    Plus,
    Redo2,
    RotateCcw,
    Scissors,
    Settings,
    Smile,
    Trash2,
    Type,
    Undo2,
    Upload,
    Volume2,
    VolumeX,
    X,
  } from "lucide-svelte";
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { save } from "@tauri-apps/plugin-dialog";
  import { get_preview_audio_url, get_static_url, invoke, listen, log, set_title, TAURI_ENV } from "../../invoker";
  import {
    clamp,
    ELEMENT_MIDDLE,
    ELEMENT_SPACE,
    newSticker,
    newText,
    styleOf,
    TEXT_STYLES,
    type CoverElement,
    type TextStyle,
  } from "../../cover-elements";
  import {
    generateEventId,
    type Config,
    type RecorderList,
    type AudioFile,
    type AudioItem,
    type ExportStarted,
    type MaterialPeaks,
    type OverlayImage,
    type OverlayItem,
    type ProjectItem,
    type ProjectTimeline,
    type SubtitleStyle,
    type TimelineClip,
    type VideoItem,
    type VideoOverlay,
  } from "../../interface";
  import {
    addAudio,
    addOverlay,
    addSubtitle,
    canShiftOverlay,
    clipToOverlay,
    clipAt,
    duplicateOverlay,
    editAudio,
    editOverlay,
    findAudio,
    findOverlay,
    frameRateQuery,
    idsAt,
    insertClips,
    mergeRemoteTimeline,
    parseSrt,
    parseTimeline,
    placeAudio,
    placeClips,
    placeOverlays,
    projectCues,
    removeAudio,
    removeClip,
    removeOverlay,
    removeSubtitle,
    setClipSubtitles,
    settleTracks,
    shiftOverlay,
    snapClips,
    splitAudio,
    splitClip,
    splitOverlay,
    timelineAudio,
    timelineDuration,
    updateSubtitleText,
    visibleSubtitles,
    MIN_OVERLAY_DURATION,
    MIN_SUBTITLE_DURATION,
    moveSubtitle,
    type AudioContent,
    type OverlayContent,
    type OverlayTarget,
  } from "../../project-timeline";
  import { ProjectPlayer, type PlayerSegment } from "../../project-player";
  import { PreviewAudio, SAMPLE_RATE, type Voice } from "../../preview-audio";
  import { shownRows } from "../../virtual-list";
  import { buildSubtitleAss, loadSubtitleStyle } from "../../subtitle-ass";
  import { formatTimecode } from "../../time-format";
  import type { RecordItem } from "../../db";
  import { platformLabel } from "../../platform";
  import {
    buildSubtitleContext,
    clampSentenceLength,
    MAX_CONTEXT_CHARS,
    SENTENCE_MAX_LENGTH,
  } from "../../powerlive-subtitle";
  import { createStateRefresh, watchStateChanges } from "../../state-refresh";
  import { announceTask, openTaskCenter, settleTask, wasCancelled } from "../../stores/tasks";
  import AssSubtitlePreview from "../AssSubtitlePreview.svelte";
  import { stickerRefUrl } from "../cover/custom-stickers";
  import ElementToolbar from "../cover/ElementToolbar.svelte";
  import { loadSticker, STICKER_GROUPS } from "../cover/render";
  import StickerPanel from "../cover/StickerPanel.svelte";
  import TextPanel from "../cover/TextPanel.svelte";
  import InspectorPanel from "../InspectorPanel.svelte";
  import ShortcutHelpModal from "../ShortcutHelpModal.svelte";
  import SubtitleStyleEditor from "../SubtitleStyleEditor.svelte";
  import TaskCenterButton from "../TaskCenterButton.svelte";
  import TitleBar from "../TitleBar.svelte";
  import AudioPanel, { type AudioEntry } from "./AudioPanel.svelte";
  import PowerLiveHotwords from "../PowerLiveHotwords.svelte";
  import PowerLiveSentenceLength from "../PowerLiveSentenceLength.svelte";
  import ItemInspector, { type InspectedItem } from "./ItemInspector.svelte";
  import MaterialBin, { type BinMaterial } from "./MaterialBin.svelte";
  import { renderOverlayImages, StickerLoadError } from "./overlay-render";
  import OverlayStage, { type OverlayChanges } from "./OverlayStage.svelte";
  import ProjectTimelineView, { type AudioDrop, type MaterialInfo } from "./ProjectTimeline.svelte";
  import "./project-editor.css";

  interface Props {
    projectId: number;
  }

  let { projectId }: Props = $props();

  const HISTORY_LIMIT = 100;
  const SAVE_DELAY_MS = 800;
  /** How long a text or sticker lasts when added. */
  const OVERLAY_DURATION = 3;
  /** A run of overlay changes, like dragging a color, closer than this is one undo step. */
  const OVERLAY_EDIT_DELAY_MS = 400;
  const PANELS = [
    { id: "materials", label: "素材", icon: Film },
    { id: "audio", label: "音频", icon: Music },
    { id: "text", label: "文字", icon: Type },
    { id: "sticker", label: "贴纸", icon: Smile },
  ] as const;

  let project: ProjectItem | null = $state(null);
  let loadError = $state("");
  let title = $state("");
  let timeline: ProjectTimeline = $state.raw(parseTimeline(""));
  let undoStack: ProjectTimeline[] = $state.raw([]);
  let redoStack: ProjectTimeline[] = $state.raw([]);
  let materials: Map<number, MaterialInfo> = $state.raw(new Map());
  let binMaterials: BinMaterial[] = $state.raw([]);
  let peaks: Map<number, MaterialPeaks> = $state.raw(new Map());
  let audioFiles: Map<number, AudioEntry> = $state.raw(new Map());
  let audioEntries: AudioEntry[] = $state.raw([]);
  let audioPeaks: Map<number, MaterialPeaks> = $state.raw(new Map());
  let danmuCounts: Map<number, number[]> = $state.raw(new Map());
  /** The frame rate export renders the project at, which edits land on; 0 until known. */
  let frameRate = $state(0);

  let time = $state(0);
  let playing = $state(false);
  /** Playing, but waiting for the sound and the picture to load first. */
  let waiting = $state(false);
  let scrubbing = false;
  let zoom = $state(1);
  let volume = $state(1);
  let muted = $state(false);
  let selectedClipId: string | null = $state(null);
  let selectedSubtitleId: string | null = $state(null);
  let selectedOverlayId: string | null = $state(null);
  let selectedAudioId: string | null = $state(null);
  let editingOverlayId: string | null = $state(null);
  let editSelectAll = $state(false);
  // What the edited text reads now, applied when editing ends.
  let editContent = "";
  // The timeline before a run of overlay changes still open as one undo step.
  let overlayEditBefore: ProjectTimeline | null = null;
  let overlayEditTimer: ReturnType<typeof setTimeout> | null = null;
  let lastStyle: TextStyle = styleOf(TEXT_STYLES[0]);
  let lastFont = "";
  let leftPanel: "materials" | "audio" | "text" | "sticker" = $state("materials");
  let stickerGroup = $state(STICKER_GROUPS[0].name);
  /** URL of each sticker reference used. */
  let stickerUrls: Map<string, string> = $state.raw(new Map());
  /** Width over height of the materials, as their videos report. */
  let aspects: Map<number, number> = $state.raw(new Map());
  let showStyleEditor = $state(false);
  let showShortcutHelp = $state(false);
  let showAiModal = $state(false);
  let aiOverwrite = $state(false);
  let aiRegenerate = $state(false);
  let aiRunning = $state(false);
  /** Whether recognizing goes through a signed-in PowerLive, which takes hints. */
  let aiPowerLive = $state(false);
  let aiHotwords: string[] = $state([]);
  let aiSentenceLength = $state(SENTENCE_MAX_LENGTH.default);
  let aiContext = $state("");
  let aiContextLoading = $state(false);
  /** Counts context fills, so only the latest one reports. */
  let aiContextFills = 0;
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
  /** Plays every sound of the preview, and is its clock. */
  let previewAudio: PreviewAudio | null = null;
  let activeVideo = $state(0);
  let previewContainer: HTMLElement = $state();
  let stage: ReturnType<typeof OverlayStage> | undefined = $state();
  let previewFrameWidth = $state(0);
  let frame = 0;
  let stopRange: (() => void) | null = null;
  let editBefore: ProjectTimeline | null = null;
  let unlistenClose: (() => void) | null = null;

  let roomId = $derived(project?.room_id ?? "");
  let duration = $derived(timelineDuration(timeline));
  let placed = $derived(placeClips(timeline));
  let subtitles = $derived(visibleSubtitles(timeline));
  // The subtitle list draws only the rows in view, and the selected one so
  // that its text stays being edited: a row per subtitle, thousands of them,
  // cost style and layout every frame while playing.
  const ROW_HEIGHT = 38;
  /** Rows drawn beyond those in view, each way. */
  const ROW_OVERSCAN = 6;
  let rowList: HTMLElement | undefined = $state();
  /** The part of the list in view, in its own pixels. */
  let listView = $state({ top: 0, height: 0 });
  let rowsShown = $derived(
    shownRows(
      listView,
      ROW_HEIGHT,
      ROW_OVERSCAN,
      subtitles.length,
      subtitles.findIndex((entry) => entry.subtitle.id === selectedSubtitleId),
    ),
  );

  $effect(() => {
    const list = rowList;
    const scroller = list?.closest<HTMLElement>(".inspector-content");
    // Measured again when the list gets longer or shorter, as its scroll may
    // change without a "scroll".
    void subtitles.length;
    if (!list || !scroller) return;
    const update = () => {
      const top = list.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
      listView = { top: scroller.scrollTop - top, height: scroller.clientHeight };
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", update);
      observer.disconnect();
    };
  });

  // The rows marked playing change only as the playhead crosses an edge: the
  // key is the same string until then, so nothing derived from it updates.
  let playingKey = $derived(idsAt(subtitles, time, (entry) => entry.subtitle.id));
  let playingSubtitles = $derived(new Set(playingKey.split("\n")));
  let hiddenSubtitleCount = $derived(timeline.subtitles.length - subtitles.length);
  let subtitleStyle: SubtitleStyle = $derived(
    (timeline.subtitle_style as SubtitleStyle | null) ?? loadSubtitleStyle(roomId),
  );
  let subtitleAss = $derived(buildSubtitleAss(projectCues(timeline, frameRate), subtitleStyle));
  let overlays = $derived(placeOverlays(timeline));
  let selectedOverlay = $derived(findOverlay(timeline, selectedOverlayId));
  let selectedPlaced = $derived(overlays.find((entry) => entry.item.id === selectedOverlayId) ?? null);
  let materialUrls = $derived(new Map([...materials].map(([id, info]) => [id, info.url])));
  let audio = $derived(placeAudio(timeline));
  let selectedAudio = $derived(findAudio(timeline, selectedAudioId));
  let selectedSound = $derived(audio.find((entry) => entry.item.id === selectedAudioId) ?? null);
  let selectedClip = $derived(placed.find((entry) => entry.clip.id === selectedClipId) ?? null);
  // What the inspector shows the properties of; with none, or a text or
  // sticker selected, it lists the subtitles.
  let inspected: InspectedItem | null = $derived.by(() => {
    if (selectedOverlay?.kind === "video" && selectedPlaced) {
      const aspect = aspects.get(selectedOverlay.material_id) ?? null;
      return { kind: "overlay", item: selectedOverlay, start: selectedPlaced.start, end: selectedPlaced.end, aspect };
    }
    if (selectedAudio && selectedSound) {
      return { kind: "audio", item: selectedAudio, start: selectedSound.start, end: selectedSound.end };
    }
    if (selectedClip) {
      const { clip, offset, duration } = selectedClip;
      return { kind: "clip", item: clip, start: offset, end: offset + duration };
    }
    return null;
  });
  // Everything heard in the preview: clips' own sound, that of video
  // overlays and the audio tracks', where each is seen or heard.
  let voices: Voice[] = $derived.by(() => {
    const result: Voice[] = [];
    for (const item of placed) {
      const file = materials.get(item.clip.material_id)?.file;
      if (!file) continue;
      const { clip, offset, duration } = item;
      result.push({ id: clip.id, file, start: offset, end: offset + duration, from: clip.start, volume: clip.volume ?? 1 });
    }
    for (const entry of overlays) {
      const item = entry.item;
      const file = item.kind === "video" ? materials.get(item.material_id)?.file : undefined;
      if (item.kind !== "video" || !file) continue;
      result.push({ id: item.id, file, start: entry.start, end: entry.end, from: item.source_start + entry.skip, volume: item.volume });
    }
    for (const entry of audio) {
      const item = entry.item;
      const file = item.kind === "file" ? audioFiles.get(item.audio_id)?.file : materials.get(item.material_id)?.file;
      if (!file) continue;
      // Shared out over the part heard, as export does.
      const length = entry.end - entry.start;
      const [fadeIn, fadeOut] = [Math.max(0, item.fade_in ?? 0), Math.max(0, item.fade_out ?? 0)];
      const scale = fadeIn + fadeOut > length ? length / (fadeIn + fadeOut) : 1;
      result.push({
        id: item.id,
        file,
        start: entry.start,
        end: entry.end,
        from: item.source_start + entry.skip,
        volume: item.volume,
        fadeIn: fadeIn * scale,
        fadeOut: fadeOut * scale,
      });
    }
    return result;
  });
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

  // What the frame rate depends on: the main track's materials and burning danmaku.
  let frameRateSource = $derived.by(() => {
    const query = frameRateQuery(timeline);
    return { query, key: `${query.materialIds.join(",")}|${query.danmu}` };
  });
  let frameRateKey = "";

  $effect(() => {
    const source = frameRateSource;
    untrack(() => void loadFrameRate(source));
  });

  /**
   * Learn the project's frame rate. Clips go on its frames with the next edit,
   * undo or merge, not here: an edit being dragged must not be saved.
   */
  async function loadFrameRate(source: { query: { materialIds: number[]; danmu: boolean }; key: string }) {
    if (source.query.materialIds.length === 0 || source.key === frameRateKey) return;
    frameRateKey = source.key;
    try {
      const rate = await invoke<number>("get_project_frame_rate", source.query);
      if (frameRateKey !== source.key) return;
      frameRate = rate;
    } catch (error) {
      if (frameRateKey === source.key) frameRateKey = "";
      console.warn("Failed to load the project's frame rate:", error);
    }
  }

  async function materialInfo(video: VideoItem): Promise<MaterialInfo> {
    return {
      id: video.id,
      name: video.title || video.file,
      url: await get_static_url("output", video.file),
      file: video.file,
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

  async function loadAudioFiles() {
    const list = await invoke<AudioFile[]>("get_audio_files");
    const entries = await Promise.all(
      list.map(async (file) => ({
        id: file.id,
        name: file.name,
        duration: file.duration,
        url: await get_static_url("output", file.file),
        file: file.file,
        createdAt: file.created_at,
      })),
    );
    audioFiles = new Map(entries.map((entry) => [entry.id, entry]));
    audioEntries = entries;
  }

  function materialName(materialId: number) {
    return materials.get(materialId)?.name ?? "素材已删除";
  }

  function soundName(item: AudioItem) {
    return item.kind === "file"
      ? (audioFiles.get(item.audio_id)?.name ?? "音频已删除")
      : `${materialName(item.material_id)}的声音`;
  }

  function inspectedName(entry: InspectedItem) {
    return entry.kind === "audio" ? soundName(entry.item) : materialName(entry.item.material_id);
  }

  /**
   * Take a saved project, merging the backend's own edits of its timeline into
   * the one edited here: danmaku switched on the uploads page, clips of a
   * deleted material, recognized subtitles. `base` is what the saved timeline
   * was made from: the one just sent when saving, or else the last one seen.
   * Returns whether the result has an edit of its own still to save.
   */
  function applyProjectView(view: ProjectItem, base?: ProjectTimeline): boolean {
    project = view;
    if (!base && view.timeline === syncedTimeline) return false;
    const from = base ?? parseTimeline(syncedTimeline);
    syncedTimeline = view.timeline;
    const merged = mergeRemoteTimeline(timeline, from, parseTimeline(view.timeline));
    // Clips that left can leave overlays on a track running into each other;
    // lifting them apart is saved like any edit, as is putting clips the
    // backend made on frames.
    const settled = snapClips(settleTracks(merged), frameRate);
    if (settled !== timeline) timeline = settled;
    return settled !== merged;
  }

  async function loadProject() {
    try {
      const view = await invoke<ProjectItem>("get_project", { id: projectId });
      // Together, so a refresh during the loads below merges into this timeline.
      const parsed = parseTimeline(view.timeline);
      let loaded = settleTracks(parsed);
      if (!loaded.subtitle_style) loaded = { ...loaded, subtitle_style: loadSubtitleStyle(view.room_id) };
      timeline = loaded;
      syncedTimeline = view.timeline;
      project = view;
      // Overlays or sounds a material deleted meanwhile left overlapping were lifted apart.
      if (loaded.tracks !== parsed.tracks || loaded.audio_tracks !== parsed.audio_tracks) scheduleSave();
      title = view.title;
      await set_title(view.title || "切片");
      await loadMaterials();
      void loadMaterialData();
    } catch (error) {
      loadError = String(error);
    }
  }

  /** Waveforms and danmaku times of the materials on the timeline, and waveforms of its sounds. */
  async function loadMaterialData() {
    const sounds = timeline.audio_tracks.flatMap((track) => track.items);
    const heard = sounds.flatMap((item) => (item.kind === "material" ? [item.material_id] : []));
    for (const id of new Set(heard)) {
      if (!peaks.has(id)) {
        invoke<MaterialPeaks>("get_material_peaks", { id })
          .then((data) => (peaks = new Map(peaks).set(id, data)))
          .catch((error) => console.warn("Failed to load waveform:", error));
      }
    }
    for (const id of timelineAudio(timeline)) {
      if (!audioPeaks.has(id)) {
        invoke<MaterialPeaks>("get_audio_peaks", { id })
          .then((data) => (audioPeaks = new Map(audioPeaks).set(id, data)))
          .catch((error) => console.warn("Failed to load waveform:", error));
      }
    }
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
    if (applyProjectView(await invoke<ProjectItem>("get_project", { id: projectId }))) scheduleSave();
  });
  // Deleted materials leave the library, and their clips the timeline.
  const materialRefresh = createStateRefresh(loadMaterials);

  onMount(() => {
    void loadProject();
    return watchStateChanges(projectRefresh, ["projects-changed"]);
  });

  onMount(() => watchStateChanges(materialRefresh, ["videos-changed"]));

  // The audio library, shared with other projects and clients.
  const audioRefresh = createStateRefresh(loadAudioFiles);
  onMount(() => {
    void audioRefresh.run();
    return watchStateChanges(audioRefresh, ["audio-changed"]);
  });

  onMount(() => {
    previewAudio = new PreviewAudio(new AudioContext({ sampleRate: SAMPLE_RATE }), async (file, chunk) => {
      const response = await fetch(await get_preview_audio_url(file, chunk));
      return response.ok ? response.arrayBuffer() : null;
    });
    previewAudio.volume = volume;
    previewAudio.muted = muted;
    previewAudio.setVoices(voices);
    player = new ProjectPlayer(videoA, videoB, previewAudio);
    const update = () => {
      if (!scrubbing) time = player.currentTime;
    };
    player.addEventListener("timeupdate", update);
    player.addEventListener("seeked", update);
    player.addEventListener("activechange", () => (activeVideo = player.activeElement === videoA ? 0 : 1));
    player.addEventListener("waiting", () => (waiting = true));
    player.addEventListener("playing", () => (waiting = false));
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
      waiting = false;
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
      finishEditing();
      settleOverlayEdit();
      await saveAll();
    });
  });

  onDestroy(() => {
    cancelAnimationFrame(frame);
    if (overlayEditTimer) clearTimeout(overlayEditTimer);
    stopRange?.();
    player?.dispose();
    previewAudio?.dispose();
    unlistenClose?.();
    if (saveTimer) void flushSave();
  });

  $effect(() => {
    segmentKey;
    if (player) untrack(() => player.setSegments(segments));
  });

  $effect(() => {
    // Read first, so the effect follows them before the player is made.
    const [level, silent] = [volume, muted];
    if (!player || !previewAudio) return;
    previewAudio.volume = level;
    previewAudio.muted = silent;
  });

  $effect(() => {
    const next = voices;
    if (player) previewAudio?.setVoices(next);
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
    let unsaved = false;
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
        unsaved = applyProjectView(view, snapshot);
        saved = true;
        saveState = editedMeanwhile || unsaved ? "pending" : "saved";
      } catch (error) {
        log.error("Failed to save project timeline:", error);
        saveState = "error";
      }
    })().finally(() => (saving = null));
    await saving;
    // Edits made during the save, or by merging its result.
    if (saved && (editedMeanwhile || unsaved)) await flushSave();
  }

  /** Apply an edit as one undo step. */
  function commit(next: ProjectTimeline, before: ProjectTimeline = timeline) {
    // A run of overlay changes still open is a step of its own before this.
    if (overlayEditBefore) settleOverlayEdit();
    // Moving or trimming clips can bring overlays or sounds on a track
    // together. Clips edited before the frame rate was known go on its frames.
    next = snapClips(settleTracks(next), frameRate);
    if (next === before) {
      timeline = before;
      return;
    }
    undoStack = [...undoStack.slice(-(HISTORY_LIMIT - 1)), before];
    redoStack = [];
    timeline = next;
    scheduleSave();
    void loadMaterialData();
    // Asked again after it failed to load.
    if (!frameRate) void loadFrameRate(frameRateSource);
  }

  function undo() {
    finishEditing();
    settleOverlayEdit();
    const previous = undoStack[undoStack.length - 1];
    if (!previous) return;
    redoStack = [...redoStack, timeline];
    undoStack = undoStack.slice(0, -1);
    // Danmaku is a setting, not an edit, so history leaves it as it is. A
    // step from before the clips were put on frames goes on them again.
    timeline = snapClips({ ...previous, danmu: timeline.danmu }, frameRate);
    scheduleSave();
  }

  function redo() {
    finishEditing();
    settleOverlayEdit();
    const next = redoStack[redoStack.length - 1];
    if (!next) return;
    undoStack = [...undoStack, timeline];
    redoStack = redoStack.slice(0, -1);
    timeline = snapClips({ ...next, danmu: timeline.danmu }, frameRate);
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
    let next = insertClips(timeline, [{ material_id: materialId, start: 0, end: info.duration }], at, frameRate);
    const added = next.clips.length > timeline.clips.length ? next.clips[at] : undefined;
    if (srt && added) next = setClipSubtitles(next, added.id, parseSrt(srt));
    commit(next);
    selectClip(added?.id ?? null);
  }

  /**
   * Select a clip of the main track, and nothing else: the inspector then
   * shows the clip, and Delete must not act on a subtitle hidden behind it.
   */
  function selectClip(id: string | null) {
    selectedClipId = id;
    if (id) {
      selectOverlay(null);
      selectedAudioId = null;
      selectedSubtitleId = null;
    }
  }

  /** Split the selected overlay or sound at the playhead, or else the clip there. */
  function splitAtPlayhead() {
    if (selectedPlaced && time > selectedPlaced.start && time < selectedPlaced.end) {
      const result = splitOverlay(timeline, selectedPlaced.item.id, time);
      if (result.id) {
        commit(result.timeline);
        selectOverlay(result.id);
      }
      return;
    }
    if (selectedSound && time > selectedSound.start && time < selectedSound.end) {
      const result = splitAudio(timeline, selectedSound.item.id, time);
      if (result.id) {
        commit(result.timeline);
        selectAudio(result.id);
      }
      return;
    }
    const result = splitClip(timeline, time, frameRate);
    if (!result.clipId) return;
    commit(result.timeline);
    selectClip(result.clipId);
  }

  function deleteSelection() {
    if (selectedOverlayId) {
      deleteOverlay(selectedOverlayId);
    } else if (selectedAudioId) {
      commit(removeAudio(timeline, selectedAudioId));
      selectedAudioId = null;
    } else if (selectedSubtitleId) {
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

  // ---- Overlays ------------------------------------------------------------

  function selectOverlay(id: string | null) {
    if (editingOverlayId && editingOverlayId !== id) finishEditing();
    selectedOverlayId = id;
    if (id) {
      selectedClipId = null;
      selectedSubtitleId = null;
      selectedAudioId = null;
    }
  }

  /** Close a run of overlay changes as one undo step. */
  function settleOverlayEdit() {
    if (overlayEditTimer) clearTimeout(overlayEditTimer);
    overlayEditTimer = null;
    const before = overlayEditBefore;
    overlayEditBefore = null;
    if (before) commit(timeline, before);
  }

  /** Apply an edit made in quick succession with others, as with a slider: together they make one undo step. */
  function editInRun(next: ProjectTimeline) {
    overlayEditBefore ??= timeline;
    timeline = next;
    scheduleSave();
    if (overlayEditTimer) clearTimeout(overlayEditTimer);
    overlayEditTimer = setTimeout(settleOverlayEdit, OVERLAY_EDIT_DELAY_MS);
  }

  /** Change an overlay; changes in quick succession make one undo step. */
  function updateOverlay(update: (item: OverlayItem) => OverlayItem, id = selectedOverlayId) {
    if (id) editInRun(editOverlay(timeline, id, update));
  }

  const withElement = (changes: OverlayChanges) => (item: OverlayItem) =>
    ({ ...item, element: { ...item.element, ...changes } }) as OverlayItem;

  /** Drags on the preview: their changes make one undo step. */
  function beginOverlayDrag() {
    if (overlayEditTimer) settleOverlayEdit();
    overlayEditBefore = timeline;
  }

  function dragOverlay(id: string, changes: OverlayChanges) {
    timeline = editOverlay(timeline, id, withElement(changes));
    scheduleSave();
  }

  function deleteOverlay(id: string) {
    if (editingOverlayId === id) editingOverlayId = null;
    commit(removeOverlay(timeline, id));
    if (selectedOverlayId === id) selectedOverlayId = null;
  }

  /** Where an overlay added now starts: at the playhead, early enough near the end to last its length. */
  function overlayStart(length = OVERLAY_DURATION) {
    return Math.max(0, Math.min(time, duration - Math.min(length, duration)));
  }

  function canAddOverlay() {
    if (duration >= MIN_OVERLAY_DURATION) return true;
    alert("请先在时间轴上添加素材，文字、贴纸和画中画叠加在素材的画面上");
    return false;
  }

  /** Texts and stickers shown at `at`, which a new one is placed apart from. */
  function elementsAt(at: number): CoverElement[] {
    return overlays
      .filter((entry) => at >= entry.start && at < entry.end && entry.item.kind !== "video")
      .map((entry) => entry.item.element as CoverElement);
  }

  /** Add an overlay as one undo step and select it. */
  function insertOverlay(content: OverlayContent, start: number, length: number, target?: OverlayTarget) {
    const result = addOverlay(timeline, content, start, length, target);
    if (!result.id) return null;
    commit(result.timeline);
    selectOverlay(result.id);
    return result.id;
  }

  function addText(style: TextStyle = lastStyle, font = lastFont) {
    finishEditing();
    if (!canAddOverlay()) return;
    const start = overlayStart();
    const element = newText(elementsAt(start), ELEMENT_MIDDLE, style, font);
    const result = addOverlay(timeline, { kind: "text", element }, start, OVERLAY_DURATION);
    if (!result.id) return;
    // Added and first typed as one undo step, as on a cover.
    if (overlayEditTimer) settleOverlayEdit();
    overlayEditBefore = timeline;
    timeline = result.timeline;
    scheduleSave();
    startEditing(result.id, true);
  }

  async function addSticker(sticker: { src: string; ref: string }, size?: number) {
    if (!canAddOverlay()) return;
    let image: HTMLImageElement;
    try {
      image = await loadSticker(sticker.src);
    } catch {
      alert("贴纸加载失败，可能已被删除");
      return;
    }
    stickerUrls = new Map(stickerUrls).set(sticker.ref, sticker.src);
    const start = overlayStart();
    const element = newSticker(elementsAt(start), ELEMENT_MIDDLE, sticker.ref, image.naturalWidth / image.naturalHeight || 1, size);
    insertOverlay({ kind: "sticker", element }, start, OVERLAY_DURATION);
  }

  function setAspect(materialId: number, aspect: number) {
    if (Number.isFinite(aspect) && aspect > 0 && aspects.get(materialId) !== aspect) {
      aspects = new Map(aspects).set(materialId, aspect);
    }
  }

  /** Width over height of a material, read from its video. */
  function probeAspect(materialId: number, url: string): Promise<number> {
    const known = aspects.get(materialId);
    if (known) return Promise.resolve(known);
    return new Promise((resolve) => {
      const video = document.createElement("video");
      const done = (aspect: number) => {
        clearTimeout(timer);
        video.removeAttribute("src");
        video.load();
        resolve(aspect);
      };
      const timer = setTimeout(() => done(16 / 9), 5000);
      video.preload = "metadata";
      video.muted = true;
      video.onloadedmetadata = () => {
        const aspect = video.videoWidth && video.videoHeight ? video.videoWidth / video.videoHeight : 16 / 9;
        setAspect(materialId, aspect);
        done(aspect);
      };
      video.onerror = () => done(16 / 9);
      video.src = url;
    });
  }

  // Materials whose ratio is being read for the inspector.
  const probingAspects = new Set<number>();

  // The inspector sizes a picture in picture by its material's ratio, which a
  // video not shown since opening has not reported.
  $effect(() => {
    if (selectedOverlay?.kind !== "video") return;
    const materialId = selectedOverlay.material_id;
    const url = materials.get(materialId)?.url;
    if (!url || probingAspects.has(materialId) || untrack(() => aspects.has(materialId))) return;
    probingAspects.add(materialId);
    void probeAspect(materialId, url).finally(() => probingAspects.delete(materialId));
  });

  /** Show a material over the main track from `start`, as a picture in the corner. */
  async function addVideoOverlay(materialId: number, start?: number, target?: OverlayTarget) {
    let info = materials.get(materialId);
    if (!info || !canAddOverlay()) return;
    if (!info.duration) {
      const video = await invoke<VideoItem>("get_video", { id: materialId });
      info = { ...info, duration: video.duration || video.length };
      materials = new Map(materials).set(materialId, info);
    }
    const aspect = await probeAspect(materialId, info.url);
    const width = Math.round(ELEMENT_SPACE.width / 3);
    const height = width / aspect;
    const margin = 48;
    const element = {
      x: Math.round(ELEMENT_SPACE.width - margin - width / 2),
      y: Math.round(ELEMENT_SPACE.height - margin - height / 2),
      width,
      rotation: 0,
    };
    // As it is after the waits, which other edits may have changed.
    const from = start ?? overlayStart(info.duration);
    insertOverlay({ kind: "video", material_id: materialId, source_start: 0, volume: 1, element }, from, info.duration, target);
  }

  /**
   * Move a clip off the main track onto a video track from `at`, filling the
   * picture as it did on the main track.
   */
  async function liftClip(clipId: string, at: number, target: OverlayTarget) {
    const clip = timeline.clips.find((item) => item.id === clipId);
    if (!clip) return;
    const url = materials.get(clip.material_id)?.url;
    const aspect = url ? await probeAspect(clip.material_id, url) : (aspects.get(clip.material_id) ?? 16 / 9);
    const width = Math.round(Math.min(ELEMENT_SPACE.width, ELEMENT_SPACE.height * aspect));
    // As it is after the wait, which other edits may have changed.
    const result = clipToOverlay(timeline, clipId, at, target, { ...ELEMENT_MIDDLE, width, rotation: 0 });
    if (!result) return;
    commit(result.timeline);
    selectOverlay(result.id);
  }

  /** Fit the selected video overlay to the whole picture. */
  async function fillFrame() {
    const item = selectedOverlay;
    if (item?.kind !== "video") return;
    const url = materials.get(item.material_id)?.url;
    // Its real ratio, which a video not shown since opening has not reported.
    const aspect = url ? await probeAspect(item.material_id, url) : (aspects.get(item.material_id) ?? 16 / 9);
    const width = Math.round(Math.min(ELEMENT_SPACE.width, ELEMENT_SPACE.height * aspect));
    updateOverlay(withElement({ ...ELEMENT_MIDDLE, width, rotation: 0 }), item.id);
    settleOverlayEdit();
  }

  function pickStyle(style: TextStyle) {
    lastStyle = styleOf(style);
    if (selectedOverlay?.kind !== "text") return addText(style);
    updateOverlay(withElement(styleOf(style)));
    settleOverlayEdit();
  }

  function pickFont(family: string) {
    lastFont = family;
    if (selectedOverlay?.kind !== "text") return addText(lastStyle, family);
    updateOverlay(withElement({ font: family }));
    settleOverlayEdit();
  }

  function duplicateSelectedOverlay() {
    if (!selectedOverlayId) return;
    // Offset, as on a cover, so the copy shows.
    const offset = (item: OverlayItem) =>
      withElement({
        x: clamp(item.element.x + 40, 0, ELEMENT_SPACE.width),
        y: clamp(item.element.y + 40, 0, ELEMENT_SPACE.height),
      })(item);
    const result = duplicateOverlay(timeline, selectedOverlayId, offset);
    if (!result.id) return;
    commit(result.timeline);
    selectOverlay(result.id);
  }

  function nudgeOverlay(dx: number, dy: number) {
    const item = selectedOverlay;
    if (!item) return;
    updateOverlay(
      withElement({
        x: clamp(item.element.x + dx, 0, ELEMENT_SPACE.width),
        y: clamp(item.element.y + dy, 0, ELEMENT_SPACE.height),
      }),
    );
  }

  function startEditing(id: string, selectAll = false) {
    if (editingOverlayId !== id) finishEditing();
    const item = findOverlay(timeline, id);
    if (item?.kind !== "text") return;
    player?.pause();
    if (overlayEditTimer) settleOverlayEdit();
    overlayEditBefore ??= timeline;
    selectedOverlayId = id;
    selectedClipId = null;
    selectedSubtitleId = null;
    editContent = item.element.content;
    editSelectAll = selectAll;
    editingOverlayId = id;
    // The playhead within it, so it shows.
    const placed = placeOverlays(timeline).find((entry) => entry.item.id === id);
    if (placed && (time < placed.start || time >= placed.end)) seek(placed.start);
  }

  /** Saved as it is typed, like a subtitle; one undo step when editing ends. */
  function typeOverlayText(content: string) {
    editContent = content;
    const item = findOverlay(timeline, editingOverlayId);
    if (item?.kind !== "text" || item.element.content === content) return;
    timeline = editOverlay(timeline, item.id, withElement({ content }));
    scheduleSave();
  }

  /** Apply what was typed, removing a text left empty, as one undo step with its start. */
  function finishEditing() {
    const id = editingOverlayId;
    if (id === null) return;
    // As on screen, also when ended by a click elsewhere mid-composition;
    // read while the field is still there.
    const content = stage?.editedText() ?? editContent;
    editingOverlayId = null;
    if (overlayEditTimer) clearTimeout(overlayEditTimer);
    overlayEditTimer = null;
    const before = overlayEditBefore ?? timeline;
    overlayEditBefore = null;
    const next = content.trim()
      ? editOverlay(timeline, id, withElement({ content }))
      : removeOverlay(timeline, id);
    if (!content.trim() && selectedOverlayId === id) selectedOverlayId = null;
    commit(next, before);
  }

  // Sticker references being resolved now.
  const resolvingStickers = new Set<string>();

  // Resolve the URLs of the stickers used. One that fails is tried again with
  // the next edit rather than kept as missing.
  $effect(() => {
    const refs = new Set<string>();
    for (const track of timeline.tracks) {
      for (const item of track.items) if (item.kind === "sticker") refs.add(item.element.src);
    }
    for (const ref of refs) {
      if (untrack(() => stickerUrls.has(ref)) || resolvingStickers.has(ref)) continue;
      resolvingStickers.add(ref);
      stickerRefUrl(ref)
        .then((url) => (stickerUrls = new Map(stickerUrls).set(ref, url)))
        .catch((error) => console.warn("Failed to resolve sticker:", error))
        .finally(() => resolvingStickers.delete(ref));
    }
  });

  // ---- Audio ---------------------------------------------------------------

  function selectAudio(id: string | null) {
    if (id && editingOverlayId) finishEditing();
    selectedAudioId = id;
    if (id) {
      selectedClipId = null;
      selectedSubtitleId = null;
      selectedOverlayId = null;
    }
  }

  /** Change a sound; changes in quick succession make one undo step. */
  function updateAudio(update: (item: AudioItem) => AudioItem, id = selectedAudioId) {
    if (id) editInRun(editAudio(timeline, id, update));
  }

  function withVolume(clip: TimelineClip, volume: number): TimelineClip {
    const { volume: _, ...rest } = clip;
    return volume === 1 ? rest : { ...rest, volume };
  }

  /** Change a clip's own sound; changes in quick succession make one undo step. */
  function setClipVolume(id: string, volume: number) {
    editInRun({ ...timeline, clips: timeline.clips.map((clip) => (clip.id === id ? withVolume(clip, volume) : clip)) });
  }

  /**
   * Play a sound on the audio tracks from `start`, or else about the
   * playhead: a file of the audio library, or the sound of a material.
   */
  async function addSound(source: AudioDrop, start?: number, target?: OverlayTarget) {
    if (duration < MIN_OVERLAY_DURATION) {
      alert("请先在时间轴上添加素材，音频跟随素材播放");
      return;
    }
    let length: number;
    let content: AudioContent;
    if (source.kind === "file") {
      const file = audioFiles.get(source.id);
      if (!file) return;
      length = file.duration;
      content = { kind: "file", audio_id: source.id, source_start: 0, volume: 1 };
    } else {
      let info = materials.get(source.id);
      if (!info) return;
      if (!info.duration) {
        const video = await invoke<VideoItem>("get_video", { id: source.id });
        info = { ...info, duration: video.duration || video.length };
        materials = new Map(materials).set(source.id, info);
      }
      length = info.duration;
      content = { kind: "material", material_id: source.id, source_start: 0, volume: 1 };
    }
    // As it is after the wait, which other edits may have changed.
    const result = addAudio(timeline, content, start ?? overlayStart(length), length, target);
    if (!result.id) return;
    commit(result.timeline);
    selectAudio(result.id);
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
    selectedOverlayId = null;
    selectedAudioId = null;
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
    // Selected as by a click, so that the inspector shows it and Delete acts
    // on what it shows.
    if (current) selectSubtitle(current.subtitle.id);
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

  async function openAiModal() {
    aiOverwrite = false;
    aiRegenerate = false;
    aiContext = "";
    try {
      const config = await invoke<Config>("get_config");
      // The web build has no PowerLive sign-in to check; its backend reports a missing one.
      aiPowerLive =
        config.subtitle_generator_type === "powerlive" &&
        (!TAURI_ENV || (await invoke<{ logged_in: boolean }>("get_powerlive_status")).logged_in);
      aiHotwords = config.powerlive_hotwords ?? [];
      aiSentenceLength = config.powerlive_sentence_max_length ?? SENTENCE_MAX_LENGTH.default;
    } catch (error) {
      console.warn("Failed to load the subtitle settings:", error);
      aiPowerLive = false;
    }
    // Opened once its layout is known; the context fills in after.
    showAiModal = true;
    if (aiPowerLive) await fillSubtitleContext();
  }

  /** Describe the recordings the clips come from, unless the user typed meanwhile. */
  async function fillSubtitleContext() {
    const fill = ++aiContextFills;
    aiContextLoading = true;
    try {
      const context = await defaultSubtitleContext();
      if (fill === aiContextFills && !aiContext.trim()) aiContext = context;
    } catch (error) {
      console.warn("Failed to build the subtitle context:", error);
    } finally {
      if (fill === aiContextFills) aiContextLoading = false;
    }
  }

  /** The streamer and stream titles of the materials, the names most likely heard. */
  async function defaultSubtitleContext() {
    const videos = await Promise.all(
      [...new Set(timeline.clips.map((clip) => clip.material_id))].map((id) =>
        invoke<VideoItem>("get_video", { id }).catch(() => null),
      ),
    );
    const sources = videos.filter((video): video is VideoItem => video !== null);
    const liveTitles = await Promise.all(
      sources
        .filter((video) => video.live_id)
        .map((video) =>
          invoke<RecordItem>("get_archive", { roomId: video.room_id, liveId: video.live_id })
            .then((record) => record.title)
            .catch(() => ""),
        ),
    );
    const room = roomId || sources[0]?.room_id || "";
    const recorders = await invoke<RecorderList>("get_recorder_list").catch(() => null);
    const recorder = recorders?.recorders.find((entry) => entry.room_info.room_id === room);
    const platform = sources.find((video) => video.platform)?.platform || recorder?.room_info.platform;
    if (!liveTitles.some(Boolean) && recorder) liveTitles.push(recorder.room_info.room_title);
    // Unknown platforms, such as imported videos, label as their id.
    const label = platform ? platformLabel(platform) : "";
    return buildSubtitleContext({
      platform: label === platform ? "" : label,
      streamer: recorder?.user_info.user_name ?? "",
      liveTitles,
      clipTitle: title,
    });
  }

  /** Recognize each material once and give its clips the cues in range. */
  async function generateSubtitles() {
    showAiModal = false;
    if (aiRunning) return;
    aiRunning = true;
    const overwrite = aiOverwrite;
    const regenerate = aiRegenerate;
    // Sent as shown: the fields' own saves are not waited for.
    const powerlive = aiPowerLive
      ? {
          context: aiContext.trim(),
          hotwords: [...aiHotwords],
          sentenceMaxLength: clampSentenceLength(aiSentenceLength),
        }
      : undefined;
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
            srt = await invoke<string>("generate_video_subtitle", { eventId, id: materialId, powerlive });
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
    finishEditing();
    settleOverlayEdit();
    await saveAll();
    await projectRefresh.run();
    // Texts and stickers are burned as drawn here, from the timeline exported.
    const exported = timeline;
    let overlayImages: OverlayImage[];
    try {
      overlayImages = await renderOverlayImages(exported, stickerRefUrl);
    } catch (error) {
      if (error instanceof StickerLoadError) {
        selectOverlay(error.overlayId);
        const placed = overlays.find((entry) => entry.item.id === error.overlayId);
        if (placed) seek(placed.start);
        alert("有贴纸没有加载出来，已选中它，请删除后再导出");
      } else {
        alert("导出失败：" + error);
      }
      return;
    }
    const exportedSubtitles = visibleSubtitles(exported).length > 0;
    // On the frames of the render, also when exporting before the editor
    // learned them.
    let fps = frameRate;
    if (!fps && exportedSubtitles) {
      const source = frameRateSource;
      try {
        fps = await invoke<number>("get_project_frame_rate", frameRateQuery(exported));
        // Kept for the edits after, as when the editor learns it itself.
        if (!frameRate && frameRateSource.key === source.key) {
          frameRate = fps;
          frameRateKey = source.key;
        }
      } catch (error) {
        console.warn("Failed to load the project's frame rate:", error);
      }
    }
    const exportedAss = buildSubtitleAss(projectCues(exported, fps), subtitleStyle);
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
        timeline: exported,
        subtitleAss: exportedSubtitles ? exportedAss : null,
        overlays: overlayImages,
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
    if (target?.closest("input, textarea, select, [contenteditable], [data-hotkey-block]") || event.altKey) {
      return;
    }
    if (event.key === "Escape" && selectedOverlayId) {
      event.preventDefault();
      selectOverlay(null);
      return;
    }
    if (event.key === "Escape" && selectedAudioId) {
      event.preventDefault();
      selectAudio(null);
      return;
    }
    if (event.key === "Escape" && selectedClipId) {
      event.preventDefault();
      selectedClipId = null;
      return;
    }
    if (event.repeat && !event.code.startsWith("Arrow")) return;
    const command = event.ctrlKey || event.metaKey;
    if (command) {
      const handled = {
        KeyZ: () => (event.shiftKey ? redo() : undo()),
        KeyY: redo,
        KeyB: splitAtPlayhead,
        KeyD: duplicateSelectedOverlay,
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
      Enter: () => {
        if (selectedOverlay?.kind === "text") startEditing(selectedOverlay.id, true);
      },
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
      <nav class="rail" aria-label="添加">
        {#each PANELS as item (item.id)}
          <button
            type="button"
            class="rail-button"
            class:active={leftPanel === item.id}
            aria-pressed={leftPanel === item.id}
            onclick={() => (leftPanel = item.id)}
          >
            <item.icon class="h-[18px] w-[18px]" />
            {item.label}
          </button>
        {/each}
      </nav>

      <!-- All stay mounted so a sticker import keeps running while another is shown. -->
      <div class="left-panel" class:hidden={leftPanel !== "materials"}>
        <MaterialBin
          materials={binMaterials}
          {roomId}
          onAdd={(material) => void addMaterial(material.id)}
          onOverlay={(material) => void addVideoOverlay(material.id)}
          onPreview={(material) => void invoke("open_clip", { videoId: material.id })}
        />
      </div>
      <div class="left-panel" class:hidden={leftPanel !== "audio"}>
        <AudioPanel files={audioEntries} onAdd={(file) => void addSound({ kind: "file", id: file.id })} />
      </div>
      <div class="left-panel element-panel custom-scrollbar-light" class:hidden={leftPanel !== "text"}>
        <TextPanel
          selected={selectedOverlay?.kind === "text" ? selectedOverlay.element : null}
          onAdd={() => addText()}
          onStyle={pickStyle}
          onFont={pickFont}
        />
      </div>
      <div class="left-panel element-panel custom-scrollbar-light" class:hidden={leftPanel !== "sticker"}>
        <StickerPanel active={leftPanel === "sticker"} bind:group={stickerGroup} onPick={addSticker} />
      </div>

      <div class="flex min-w-0 flex-1 flex-col">
        <div class="overlay-toolbar">
          {#if selectedOverlay?.kind === "text" || selectedOverlay?.kind === "sticker"}
            <ElementToolbar
              text={selectedOverlay.kind === "text" ? selectedOverlay.element : null}
              sticker={selectedOverlay.kind === "sticker" ? selectedOverlay.element : null}
              onUpdate={(changes) => updateOverlay(withElement(changes))}
              onFont={pickFont}
            />
          {:else if selectedOverlay?.kind === "video"}
            <span class="truncate text-[11px] text-fg-muted">画中画 · {materialName(selectedOverlay.material_id)}</span>
          {:else if selectedAudio}
            <span class="truncate text-[11px] text-fg-muted">音频 · {soundName(selectedAudio)}</span>
          {:else if selectedClip}
            <span class="truncate text-[11px] text-fg-muted">主轨片段 · {materialName(selectedClip.clip.material_id)}</span>
          {:else}
            <span class="truncate text-[12px] text-fg-muted">
              从左侧添加文字、贴纸，或把素材拖到时间轴主轨上方的视频轨道上叠加画面；点击画面中的内容可移动、旋转、缩放，双击文字可直接修改。
            </span>
          {/if}

          {#if selectedAudio}
            <div class="ml-auto flex shrink-0 items-center gap-1">
              {#if selectedSound}
                <span class="mr-1 text-[11px] tabular-nums text-fg-muted">
                  {formatTimecode(selectedSound.start)} – {formatTimecode(selectedSound.end)}
                </span>
              {/if}
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost hover:text-danger-fg"
                title="删除 (Delete)"
                aria-label="删除"
                onclick={deleteSelection}
              >
                <Trash2 class="h-3.5 w-3.5" />
              </button>
            </div>
          {/if}

          {#if selectedOverlay}
            <div class="ml-auto flex shrink-0 items-center gap-1">
              {#if selectedPlaced}
                <span class="mr-1 text-[11px] tabular-nums text-fg-muted">
                  {formatTimecode(selectedPlaced.start)} – {formatTimecode(selectedPlaced.end)}
                </span>
              {/if}
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="上移一层"
                aria-label="上移一层"
                disabled={!canShiftOverlay(timeline, selectedOverlay.id, 1)}
                onclick={() => commit(shiftOverlay(timeline, selectedOverlay.id, 1))}
              >
                <ArrowUp class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="下移一层"
                aria-label="下移一层"
                disabled={!canShiftOverlay(timeline, selectedOverlay.id, -1)}
                onclick={() => commit(shiftOverlay(timeline, selectedOverlay.id, -1))}
              >
                <ArrowDown class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="复制 (⌘/Ctrl+D)"
                aria-label="复制"
                onclick={duplicateSelectedOverlay}
              >
                <Copy class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost hover:text-danger-fg"
                title="删除 (Delete)"
                aria-label="删除"
                onclick={() => deleteOverlay(selectedOverlay.id)}
              >
                <Trash2 class="h-3.5 w-3.5" />
              </button>
            </div>
          {/if}
        </div>

        <div class="relative min-h-0 flex-1 bg-black">
          <div bind:this={previewContainer} class="absolute inset-0 flex items-center justify-center">
            <div class="relative aspect-video bg-black" style:width="{previewFrameWidth}px">
              <!-- svelte-ignore a11y_media_has_caption -->
              <video bind:this={videoA} class="preview-video" class:visible={activeVideo === 0}></video>
              <!-- svelte-ignore a11y_media_has_caption -->
              <video bind:this={videoB} class="preview-video" class:visible={activeVideo === 1}></video>
              <OverlayStage
                bind:this={stage}
                placed={overlays}
                trackCount={timeline.tracks.length}
                {time}
                {playing}
                {waiting}
                width={previewFrameWidth}
                selectedId={selectedOverlayId}
                editingId={editingOverlayId}
                {editSelectAll}
                {materialUrls}
                {stickerUrls}
                {aspects}
                onSelect={selectOverlay}
                onBackground={togglePlay}
                onDragStart={beginOverlayDrag}
                onChange={dragOverlay}
                onDragEnd={settleOverlayEdit}
                onDelete={deleteOverlay}
                onStartEditing={(id) => startEditing(id, true)}
                onEditInput={typeOverlayText}
                onEditDone={() => finishEditing()}
                onNudge={nudgeOverlay}
                onAspect={setAspect}
              >
                {#snippet above()}
                  {#if player && subtitles.length > 0}
                    <AssSubtitlePreview assContent={subtitleAss} video={player} fontName={subtitleStyle.fontName} />
                  {/if}
                {/snippet}
              </OverlayStage>
              {#if timeline.clips.length === 0 && project}
                <div class="empty-preview">
                  <Film class="h-8 w-8 text-gray-600" />
                  <span>从左侧素材库添加素材开始剪辑</span>
                </div>
              {/if}
            </div>
          </div>
        </div>
      </div>

      <InspectorPanel
        title={inspected ? "属性" : `字幕${subtitles.length > 0 ? ` · ${subtitles.length}` : ""}`}
        tabs={[{ id: "inspect", label: "检视" }]}
        activeTab="inspect"
        collapsed={false}
        docked
      >
        {#if inspected}
          {@const id = inspected.item.id}
          <ItemInspector
            {inspected}
            name={inspectedName(inspected)}
            onClipVolume={(volume) => setClipVolume(id, volume)}
            onOverlay={(update) => updateOverlay((item) => update(item as VideoOverlay), id)}
            onAudio={(update) => updateAudio(update, id)}
            onSettle={settleOverlayEdit}
            onFillFrame={() => void fillFrame()}
          />
        {:else}
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
              onclick={openAiModal}
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
            <!-- One pixel short: the last row's border falls on the list's own. -->
            <div class="row-list virtual" bind:this={rowList} style:height="{subtitles.length * ROW_HEIGHT - 1}px">
              {#each rowsShown as index (subtitles[index].subtitle.id)}
                {@const entry = subtitles[index]}
                <div
                  class="row"
                  style:top="{index * ROW_HEIGHT}px"
                  data-subtitle-row={entry.subtitle.id}
                  class:playing={playingSubtitles.has(entry.subtitle.id)}
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
        {/if}
      </InspectorPanel>
    </div>

    <div class="transport">
      <div class="flex items-center gap-1">
        <button
          class="icon-button"
          title="在播放头处分割选中的叠加内容、音频或片段 (⌘B)"
          disabled={!clipAt(timeline, time)}
          onclick={splitAtPlayhead}
        >
          <Scissors class="h-4 w-4" />
          分割
        </button>
        <button
          class="icon-button"
          title="删除选中 (Delete)"
          disabled={!selectedClipId && !selectedSubtitleId && !selectedOverlayId && !selectedAudioId}
          onclick={deleteSelection}
        >
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
        <span class="clock">{formatTimecode(time)} / {formatTimecode(duration)}</span>
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
      {audioFiles}
      {audioPeaks}
      {frameRate}
      {time}
      {zoom}
      {selectedClipId}
      {selectedSubtitleId}
      {selectedOverlayId}
      {selectedAudioId}
      {stickerUrls}
      onSeek={seek}
      onScrub={(active) => (scrubbing = active)}
      onPreview={(next) => (timeline = next)}
      onCommit={(next, before) => commit(next, before)}
      onSelectClip={selectClip}
      onSelectSubtitle={(id) => {
        if (id) selectSubtitle(id);
        else selectedSubtitleId = null;
      }}
      onSelectOverlay={selectOverlay}
      onSelectAudio={selectAudio}
      onDropMaterial={(id, index) => void addMaterial(id, index)}
      onDropOverlay={(id, at, target) => void addVideoOverlay(id, at, target)}
      onLiftClip={(id, at, target) => void liftClip(id, at, target)}
      onDropAudio={(source, at, target) => void addSound(source, at, target)}
      onZoom={(value) => (zoom = value)}
    />
  </div>
</div>

{#if showAiModal}
  <div class="ui-modal-backdrop z-[1100]">
    <div
      class="ui-modal max-w-full {aiPowerLive ? 'w-[520px]' : 'w-[440px]'}"
      role="dialog"
      aria-modal="true"
      aria-labelledby="ai-subtitle-title"
    >
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
        {#if aiPowerLive}
          <div class="space-y-3 border-t border-line pt-4">
            <p class="text-[11px] leading-relaxed text-fg-muted">
              PowerLive 会参考热词和上下文识别专有名词，并在识别后校对同音字。已识别过的素材直接使用缓存，修改后需勾选「忽略缓存，重新识别」才会生效。
            </p>
            <div>
              <div class="mb-1.5 flex items-baseline justify-between gap-3">
                <span class="ui-label">热词</span>
                <span class="text-[11px] text-fg-muted">全局设置，所有切片共用</span>
              </div>
              <PowerLiveHotwords bind:hotwords={aiHotwords} />
            </div>
            <div class="flex items-center justify-between gap-3">
              <label for="ai-subtitle-sentence-length" class="ui-label">
                每条字幕最多字数
                <span class="ml-1.5 text-[11px] font-normal text-fg-muted">全局设置，范围 4–50</span>
              </label>
              <PowerLiveSentenceLength id="ai-subtitle-sentence-length" bind:value={aiSentenceLength} />
            </div>
            <div>
              <div class="mb-1.5 flex items-center justify-between gap-3">
                <label for="ai-subtitle-context" class="ui-label">上下文</label>
                <button
                  type="button"
                  class="ui-btn ui-btn-sm ui-btn-ghost -mr-1.5"
                  disabled={aiContextLoading}
                  onclick={() => {
                    aiContext = "";
                    void fillSubtitleContext();
                  }}
                >
                  {#if aiContextLoading}
                    <Loader2 class="h-3 w-3 animate-spin" />
                  {:else}
                    <RotateCcw class="h-3 w-3" />
                  {/if}
                  按素材重新生成
                </button>
              </div>
              <textarea
                id="ai-subtitle-context"
                class="ui-input w-full resize-none"
                rows="3"
                maxlength={MAX_CONTEXT_CHARS}
                bind:value={aiContext}
                placeholder={aiContextLoading ? "正在根据素材生成…" : "一两句包含专有名词的背景，如主播名、直播标题和在玩的游戏"}
                data-hotkey-block
              ></textarea>
              <div class="mt-1 flex justify-between gap-3 text-[11px]">
                <span class="text-fg-muted">只用于这次识别，默认根据主播名和直播标题生成</span>
                <span class="shrink-0 text-fg-faint">{[...aiContext].length} / {MAX_CONTEXT_CHARS}</span>
              </div>
            </div>
          </div>
        {/if}
      </div>
      <div class="ui-modal-footer">
        <button type="button" class="ui-btn" onclick={() => (showAiModal = false)}>取消</button>
        <!-- The context is part of the request, so wait until it is filled. -->
        <button type="button" class="ui-btn ui-btn-primary" disabled={aiContextLoading} onclick={generateSubtitles}>
          开始识别
        </button>
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
  }

  .preview-video.visible {
    visibility: visible;
  }

  .empty-preview {
    position: absolute;
    inset: 0;
    z-index: 100;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: #6b7280;
    font-size: 13px;
  }

  .rail {
    display: flex;
    width: 56px;
    flex-shrink: 0;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    border-right: 1px solid #222a35;
    background: #11151c;
    padding: 10px 0;
  }

  .rail-button {
    display: flex;
    width: 44px;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    border-radius: 8px;
    padding: 7px 0;
    color: #8c98aa;
    font-size: 11px;
    transition: background-color 150ms ease, color 150ms ease;
  }

  .rail-button:hover {
    background: rgb(255 255 255 / 5%);
    color: #dce5f3;
  }

  .rail-button.active {
    background: rgb(10 132 255 / 18%);
    color: #55b9ff;
  }

  .left-panel {
    display: flex;
    flex-shrink: 0;
  }

  .left-panel.hidden {
    display: none;
  }

  .element-panel {
    width: 252px;
    flex-direction: column;
    overflow-y: auto;
    border-right: 1px solid #222a35;
    background: #171c25;
    padding: 12px;
  }

  .overlay-toolbar {
    display: flex;
    min-height: 42px;
    flex-shrink: 0;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    border-bottom: 1px solid #222a35;
    background: #171c25;
    padding: 6px 10px;
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
