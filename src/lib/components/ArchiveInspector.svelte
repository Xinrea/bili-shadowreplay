<script lang="ts">
  import {
    Check,
    ChevronRight,
    Download,
    ExternalLink,
    Search,
    Trash2,
  } from "lucide-svelte";
  import type { RecordItem } from "../db";
  import type { DanmuEntry, Marker, Range, VideoItem } from "../interface";
  import ArchiveClipButton from "./ArchiveClipButton.svelte";
  import MarkerPanel from "./MarkerPanel.svelte";

  type InspectorTab = "ranges" | "danmu" | "markers" | "clips";
  type DanmuPeak = {
    start: number;
    end: number;
    count: number;
    exists: boolean;
  };
  type GeneratedVideo = {
    id: number;
    value: number;
    name: string;
    file: string;
    cover: string;
  };

  interface Props {
    archive?: RecordItem | null;
    ranges?: Range[];
    markers?: Marker[];
    danmuRecords?: DanmuEntry[];
    globalOffset?: number;
    danmuPeaks?: DanmuPeak[];
    peakThreshold?: number;
    videos?: GeneratedVideo[];
    selectedVideo?: GeneratedVideo | null;
    clipRunning?: boolean;
    selectedRangeIndex?: number;
    onCollapse?: () => void;
    onPeakThresholdChange?: (value: number) => void;
    onSeek?: (seconds: number) => void;
    onAddPeak?: (peak: DanmuPeak) => void;
    onAddAllPeaks?: () => void;
    onVideoSelect?: (id: number) => void;
    onDeleteVideo?: () => void;
    onDownloadVideo?: () => void;
    onOpenVideo?: (id: number) => void;
    onGenerated?: (video: VideoItem) => void;
  }

  let {
    archive = null,
    ranges = $bindable([]),
    markers = $bindable([]),
    danmuRecords = [],
    globalOffset = 0,
    danmuPeaks = [],
    peakThreshold = 80,
    videos = [],
    selectedVideo = null,
    clipRunning = $bindable(false),
    selectedRangeIndex = $bindable(-1),
    onCollapse,
    onPeakThresholdChange,
    onSeek,
    onAddPeak,
    onAddAllPeaks,
    onVideoSelect,
    onDeleteVideo,
    onDownloadVideo,
    onOpenVideo,
    onGenerated,
  }: Props = $props();

  let activeTab: InspectorTab = $state("ranges");
  let danmuSearch = $state("");
  let danmuScrollTop = $state(0);
  let danmuViewportHeight = $state(480);
  const DANMU_ITEM_HEIGHT = 66;
  const DANMU_BUFFER = 8;

  let activeRanges = $derived(
    ranges.filter((range) => range.activated !== false),
  );
  let activeDuration = $derived(
    activeRanges.reduce(
      (total, range) => total + Math.max(0, range.end - range.start),
      0,
    ),
  );
  let filteredDanmu = $derived(
    danmuSearch.trim()
      ? danmuRecords.filter((entry) =>
          entry.content.toLowerCase().includes(danmuSearch.trim().toLowerCase()),
        )
      : danmuRecords,
  );
  let visibleDanmuStart = $derived(
    Math.max(0, Math.floor(danmuScrollTop / DANMU_ITEM_HEIGHT) - DANMU_BUFFER),
  );
  let visibleDanmuEnd = $derived(
    Math.min(
      filteredDanmu.length,
      Math.ceil(
        (danmuScrollTop + danmuViewportHeight) / DANMU_ITEM_HEIGHT,
      ) + DANMU_BUFFER,
    ),
  );

  const tabs: { id: InspectorTab; label: string }[] = [
    { id: "ranges", label: "选区" },
    { id: "danmu", label: "弹幕" },
    { id: "markers", label: "标记" },
    { id: "clips", label: "切片" },
  ];

  function formatTime(seconds: number) {
    const total = Math.max(0, Math.floor(seconds || 0));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const rest = total % 60;
    return [hours, minutes, rest]
      .map((part) => part.toString().padStart(2, "0"))
      .join(":");
  }

  function toggleRange(index: number, active: boolean) {
    ranges = ranges.map((range, rangeIndex) =>
      rangeIndex === index ? { ...range, activated: active } : range,
    );
  }

  function removeRange(index: number) {
    ranges = ranges.filter((_, rangeIndex) => rangeIndex !== index);
    if (selectedRangeIndex === index) selectedRangeIndex = -1;
    if (selectedRangeIndex > index) selectedRangeIndex -= 1;
  }

  function selectRange(index: number, range: Range) {
    selectedRangeIndex = index;
    onSeek?.(range.start);
  }
</script>

<aside class="inspector">
  <div class="inspector-heading">
    <h2>素材检查器</h2>
    <button type="button" title="收起检查器" onclick={onCollapse}>
      <ChevronRight size={17} />
    </button>
  </div>

  <div class="tabs" role="tablist" aria-label="素材分类">
    {#each tabs as tab}
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === tab.id}
        class:active={activeTab === tab.id}
        onclick={() => (activeTab = tab.id)}
      >
        {tab.label}
        {#if tab.id === "ranges" && activeRanges.length > 0}
          <span>{activeRanges.length}</span>
        {/if}
      </button>
    {/each}
  </div>

  <div class="inspector-content">
    {#if activeTab === "ranges"}
      <section class="range-summary">
        <div class="summary-value">
          <span>已选内容</span>
          <strong>{formatTime(activeDuration)}</strong>
        </div>
        <label
          class="select-all-toggle"
          class:checked={ranges.length > 0 &&
            ranges.every((range) => range.activated !== false)}
        >
          <input
            type="checkbox"
            checked={ranges.length > 0 &&
              ranges.every((range) => range.activated !== false)}
            onchange={(event) => {
              const active = (event.currentTarget as HTMLInputElement).checked;
              ranges = ranges.map((range) => ({ ...range, activated: active }));
            }}
          />
          <span class="check-mark">
            {#if ranges.length > 0 &&
              ranges.every((range) => range.activated !== false)}
              <Check size={13} />
            {/if}
          </span>
          全选
        </label>
      </section>

      <section class="range-list" aria-label="选区列表">
        {#each ranges as range, index}
          <article
            class="range-card"
            class:selected={selectedRangeIndex === index}
            class:inactive={range.activated === false}
          >
            <button
              type="button"
              class="range-main"
              onclick={() => selectRange(index, range)}
            >
              <span class="range-title">{index + 1}</span>
              <span class="range-time">
                {formatTime(range.start)} → {formatTime(range.end)}
              </span>
              <span class="range-duration">
                {formatTime(range.end - range.start)}
              </span>
            </button>
            <div class="range-actions">
              <label
                class="range-toggle"
                class:checked={range.activated !== false}
                title={range.activated === false ? "启用选区" : "停用选区"}
              >
                <input
                  type="checkbox"
                  checked={range.activated !== false}
                  onchange={(event) =>
                    toggleRange(
                      index,
                      (event.currentTarget as HTMLInputElement).checked,
                    )}
                />
                <span class="check-mark">
                  {#if range.activated !== false}<Check size={13} />{/if}
                </span>
              </label>
              <button
                type="button"
                title="删除选区"
                onclick={() => removeRange(index)}
              >
                <Trash2 size={13} />
              </button>
            </div>
          </article>
        {:else}
          <div class="empty-state">
            <strong>还没有选区</strong>
            <span>在下方时间线点击“新建选区”开始剪辑</span>
          </div>
        {/each}
      </section>

      <section class="recommendations">
        <div class="section-heading">
          <div>
            <h3>智能推荐</h3>
            <span>基于弹幕热度发现精彩片段</span>
          </div>
          {#if danmuPeaks.some((peak) => !peak.exists)}
            <button type="button" onclick={onAddAllPeaks}>全部添加</button>
          {/if}
        </div>
        <label class="threshold">
          <span>峰值阈值 {peakThreshold}%</span>
          <input
            type="range"
            min="50"
            max="100"
            value={peakThreshold}
            oninput={(event) =>
              onPeakThresholdChange?.(
                Number((event.currentTarget as HTMLInputElement).value),
              )}
          />
        </label>
        <div class="peak-list">
          {#each danmuPeaks as peak}
            <button
              type="button"
              class:exists={peak.exists}
              onclick={() => {
                if (peak.exists) onSeek?.(peak.start);
                else onAddPeak?.(peak);
              }}
            >
              <span>
                {formatTime(peak.start)} → {formatTime(peak.end)}
                <small>{peak.count} 条弹幕</small>
              </span>
              <b>{peak.exists ? "已存在" : "+ 添加"}</b>
            </button>
          {:else}
            <p class="empty-copy">
              {danmuRecords.length === 0
                ? "暂无弹幕数据"
                : "当前阈值下未发现峰值"}
            </p>
          {/each}
        </div>
      </section>
    {:else if activeTab === "danmu"}
      <section class="danmu-panel">
        <label class="search-box">
          <Search size={15} />
          <input bind:value={danmuSearch} placeholder="搜索弹幕内容" />
        </label>
        <div class="result-count">
          共 {danmuRecords.length} 条，显示 {filteredDanmu.length} 条
        </div>
        <div
          class="danmu-list"
          onscroll={(event) => {
            const element = event.currentTarget as HTMLElement;
            danmuScrollTop = element.scrollTop;
            danmuViewportHeight = element.clientHeight;
          }}
        >
          <div
            style:height={`${visibleDanmuStart * DANMU_ITEM_HEIGHT}px`}
          ></div>
          {#each filteredDanmu.slice(visibleDanmuStart, visibleDanmuEnd) as danmu}
            <button
              type="button"
              class="danmu-entry"
              onclick={() => onSeek?.(danmu.ts / 1000 - globalOffset)}
            >
              <span>{danmu.content}</span>
              <time>{formatTime(danmu.ts / 1000 - globalOffset)}</time>
            </button>
          {:else}
            <div class="empty-state">
              <strong>没有匹配的弹幕</strong>
            </div>
          {/each}
          <div
            style:height={`${Math.max(
              0,
              filteredDanmu.length - visibleDanmuEnd,
            ) * DANMU_ITEM_HEIGHT}px`}
          ></div>
        </div>
      </section>
    {:else if activeTab === "markers"}
      <MarkerPanel
        {archive}
        bind:markers
        embedded
        onMarkerClick={(marker) => onSeek?.(marker.offset)}
      />
    {:else}
      <section class="clips-panel">
        <div class="section-heading">
          <div>
            <h3>已生成切片</h3>
            <span>当前直播间下共 {videos.length} 个切片</span>
          </div>
        </div>
        <div class="clip-list">
          {#each videos as item}
            <article
              class="clip-card"
              class:selected={selectedVideo?.id === item.id}
            >
              <button
                type="button"
                class="clip-preview"
                onclick={() => onVideoSelect?.(item.id)}
              >
                {#if item.cover}
                  <img src={item.cover} alt="" />
                {:else}
                  <span>暂无封面</span>
                {/if}
              </button>
              <div class="clip-info">
                <button type="button" onclick={() => onVideoSelect?.(item.id)}>
                  {item.name}
                </button>
                <div>
                  <button
                    type="button"
                    title="打开切片"
                    onclick={() => onOpenVideo?.(item.id)}
                  >
                    <ExternalLink size={13} />
                  </button>
                  {#if selectedVideo?.id === item.id}
                    <button
                      type="button"
                      title="下载切片"
                      onclick={onDownloadVideo}
                    >
                      <Download size={13} />
                    </button>
                    <button
                      type="button"
                      title="删除切片"
                      class="danger"
                      onclick={onDeleteVideo}
                    >
                      <Trash2 size={13} />
                    </button>
                  {/if}
                </div>
              </div>
            </article>
          {:else}
            <div class="empty-state">
              <strong>还没有生成切片</strong>
              <span>启用选区后可在下方合成为一个切片</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  </div>

  <footer class="inspector-footer">
    <div>
      <span>{activeRanges.length} 个选区</span>
      <strong>合成后约 {formatTime(activeDuration)}</strong>
    </div>
    <ArchiveClipButton
      {archive}
      ranges={activeRanges}
      captureCover
      bind:running={clipRunning}
      onGenerated={onGenerated}
    />
  </footer>
</aside>

<style>
  .inspector {
    display: flex;
    width: 340px;
    height: 100%;
    min-height: 0;
    flex-direction: column;
    border-left: 1px solid #2b3340;
    background: #171c25;
    color: #e9eef8;
  }

  .inspector-heading {
    display: flex;
    height: 50px;
    flex: 0 0 50px;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 0 18px;
  }

  .inspector-heading h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 650;
  }

  .inspector-heading button,
  .range-actions button,
  .range-actions label,
  .clip-info div button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    color: #8c98aa;
  }

  .inspector-heading button {
    width: 28px;
    height: 28px;
    background: #222a36;
  }

  .tabs {
    display: grid;
    margin: 0 16px 14px;
    grid-template-columns: repeat(4, 1fr);
    border-radius: 9px;
    background: #0f131a;
    padding: 3px;
  }

  .tabs button {
    display: flex;
    height: 31px;
    align-items: center;
    justify-content: center;
    gap: 5px;
    border-radius: 7px;
    color: #8490a3;
    font-size: 11px;
  }

  .tabs button.active {
    background: #293341;
    color: #edf4ff;
  }

  .tabs button span {
    min-width: 16px;
    border-radius: 999px;
    background: #168de0;
    padding: 1px 5px;
    color: white;
    font-size: 9px;
  }

  .inspector-content {
    min-height: 0;
    flex: 1;
    overflow-y: auto;
    padding: 0 16px 16px;
  }

  .range-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-radius: 10px;
    background: #202733;
    padding: 11px 13px;
  }

  .range-summary div {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .range-summary span,
  .range-summary label,
  .section-heading span,
  .result-count {
    color: #8490a3;
    font-size: 10px;
  }

  .range-summary strong {
    font-size: 16px;
    font-weight: 600;
  }

  .select-all-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }

  .select-all-toggle input,
  .range-toggle input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
  }

  .range-list,
  .peak-list,
  .clip-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
  }

  .range-card {
    display: flex;
    min-height: 46px;
    align-items: center;
    border: 1px solid transparent;
    border-radius: 10px;
    background: #202733;
    transition:
      border-color 150ms ease,
      opacity 150ms ease;
  }

  .range-card.selected {
    border-color: #35a7ff;
    box-shadow: inset 3px 0 #35a7ff;
  }

  .range-card.inactive {
    opacity: 0.55;
  }

  .range-main {
    display: flex;
    height: 44px;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 10px;
    overflow: hidden;
    padding: 0 12px;
    text-align: left;
  }

  .range-title {
    width: 20px;
    flex: 0 0 20px;
    overflow: hidden;
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .range-time {
    min-width: 0;
    overflow: hidden;
    flex: 1 1 auto;
    color: #b6c0cf;
    font-size: 10px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .range-duration {
    flex: 0 0 auto;
    color: #718095;
    font-size: 10px;
    white-space: nowrap;
  }

  .range-actions {
    display: flex;
    gap: 3px;
    padding-right: 9px;
  }

  .range-actions button,
  .range-actions label {
    width: 22px;
    height: 22px;
    background: #171c25;
    cursor: pointer;
  }

  .clip-info div button {
    width: 27px;
    height: 27px;
    background: #171c25;
    cursor: pointer;
  }

  .range-actions label input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
  }

  .check-mark {
    display: inline-flex;
    width: 22px;
    height: 22px;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    background: #171c25;
    color: #8290a4;
    transition:
      background-color 150ms ease,
      color 150ms ease;
  }

  .select-all-toggle .check-mark {
    width: 20px;
    height: 20px;
    border: 1px solid #435064;
    border-radius: 5px;
  }

  .select-all-toggle.checked .check-mark,
  .range-toggle.checked .check-mark {
    border-color: #0a84ff;
    background: #0a84ff;
    color: white;
  }

  .recommendations {
    margin-top: 18px;
  }

  .section-heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
  }

  .section-heading div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .section-heading h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }

  .section-heading button,
  .peak-list b {
    color: #55b9ff;
    font-size: 10px;
    font-weight: 500;
  }

  .threshold {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
    border-radius: 8px;
    background: #202733;
    padding: 8px 10px;
    color: #9aa6b8;
    font-size: 10px;
  }

  .threshold input {
    min-width: 0;
    flex: 1;
    accent-color: #0a84ff;
  }

  .peak-list button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: 1px solid #2d3745;
    border-radius: 8px;
    background: #1c222d;
    padding: 8px 10px;
    text-align: left;
  }

  .peak-list button > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 10px;
  }

  .peak-list small {
    color: #718095;
    font-size: 9px;
  }

  .peak-list button.exists {
    opacity: 0.65;
  }

  .empty-copy {
    margin: 4px 0;
    color: #718095;
    font-size: 10px;
  }

  .danmu-panel,
  .clips-panel {
    display: flex;
    height: 100%;
    min-height: 0;
    flex-direction: column;
  }

  .search-box {
    display: flex;
    height: 36px;
    flex: 0 0 36px;
    align-items: center;
    gap: 8px;
    border: 1px solid #303947;
    border-radius: 9px;
    background: #202733;
    padding: 0 10px;
    color: #718095;
  }

  .search-box:focus-within {
    border-color: #0a84ff;
  }

  .search-box input {
    min-width: 0;
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: #eef4fd;
    font-size: 11px;
  }

  .result-count {
    padding: 9px 2px;
  }

  .danmu-list {
    min-height: 0;
    flex: 1;
    overflow-y: auto;
  }

  .danmu-entry {
    display: flex;
    width: 100%;
    height: 42px;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 8px;
    border: 1px solid #293442;
    border-radius: 9px;
    background: #202733;
    padding: 8px 10px;
    text-align: left;
  }

  .danmu-entry span {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    color: #d8e0eb;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .danmu-entry time {
    flex: 0 0 auto;
    color: #718095;
    font-size: 9px;
  }

  .clip-card {
    display: grid;
    overflow: hidden;
    grid-template-columns: 94px minmax(0, 1fr);
    border: 1px solid #293442;
    border-radius: 9px;
    background: #202733;
  }

  .clip-card.selected {
    border-color: #35a7ff;
  }

  .clip-preview {
    display: flex;
    min-height: 60px;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background: #0c1016;
    color: #657186;
    font-size: 9px;
  }

  .clip-preview img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .clip-info {
    display: flex;
    min-width: 0;
    flex-direction: column;
    justify-content: space-between;
    padding: 8px;
  }

  .clip-info > button {
    overflow: hidden;
    color: #dce5f3;
    font-size: 10px;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .clip-info div {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
  }

  .clip-info div button.danger,
  .range-actions button:hover {
    color: #ff7181;
  }

  .empty-state {
    display: flex;
    min-height: 120px;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    gap: 5px;
    color: #718095;
    text-align: center;
  }

  .empty-state strong {
    color: #a8b3c3;
    font-size: 12px;
    font-weight: 500;
  }

  .empty-state span {
    font-size: 10px;
  }

  .inspector-footer {
    display: flex;
    min-height: 66px;
    flex: 0 0 66px;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border-top: 1px solid #2b3340;
    background: #141922;
    padding: 10px 16px;
  }

  .inspector-footer > div:first-child {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
  }

  .inspector-footer span {
    color: #7f8b9d;
    font-size: 9px;
  }

  .inspector-footer strong {
    color: #dce5f3;
    font-size: 10px;
    font-weight: 500;
    white-space: nowrap;
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }
</style>
