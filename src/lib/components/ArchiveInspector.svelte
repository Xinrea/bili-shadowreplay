<script lang="ts">
  import {
    Check,
    ChevronRight,
    Download,
    ExternalLink,
    Filter,
    Search,
    Trash2,
  } from "lucide-svelte";
  import type { RecordItem } from "../db";
  import type { DanmuEntry, Marker, Range, VideoItem } from "../interface";
  import { onDestroy } from "svelte";
  import ArchiveClipButton from "./ArchiveClipButton.svelte";
  import MarkerPanel from "./MarkerPanel.svelte";
  import { TAURI_ENV } from "../invoker";
  import { clickOutside } from "../actions/clickOutside";
  import { virtualListWindow } from "../live-preview-perf";
  import { formatScPrice, getScColors, isSuperChat } from "../superchat";

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
    danmuKeywords?: string[];
    videos?: GeneratedVideo[];
    selectedVideo?: GeneratedVideo | null;
    clipRunning?: boolean;
    selectedRangeIndex?: number;
    onCollapse?: () => void;
    onPeakThresholdChange?: (value: number) => void;
    onDanmuKeywordsChange?: (keywords: string[]) => void;
    onSeek?: (seconds: number) => void;
    onAddPeak?: (peak: DanmuPeak) => void;
    onAddAllPeaks?: () => void;
    onVideoSelect?: (id: number) => void;
    onDeleteVideo?: (id: number) => void;
    onDownloadVideo?: (id: number) => void;
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
    danmuKeywords = [],
    videos = [],
    selectedVideo = null,
    clipRunning = $bindable(false),
    selectedRangeIndex = $bindable(-1),
    onCollapse,
    onPeakThresholdChange,
    onDanmuKeywordsChange,
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
  let danmuListEl: HTMLElement | null = $state(null);
  let scTooltip = $state<{ entry: DanmuEntry; x: number; y: number } | null>(
    null,
  );
  const SC_TOOLTIP_WIDTH = 260;
  const SC_TOOLTIP_ESTIMATED_HEIGHT = 110;

  function showScTooltip(entry: DanmuEntry, element: HTMLElement) {
    const rect = element.getBoundingClientRect();
    // Anchor the tooltip's right edge to the entry so it stays inside the
    // inspector panel, flipping above the entry when it would overflow down.
    const x = Math.max(
      8,
      Math.min(
        rect.right - SC_TOOLTIP_WIDTH,
        window.innerWidth - SC_TOOLTIP_WIDTH - 8,
      ),
    );
    const belowY = rect.bottom + 6;
    const y =
      belowY + SC_TOOLTIP_ESTIMATED_HEIGHT > window.innerHeight
        ? Math.max(8, rect.top - SC_TOOLTIP_ESTIMATED_HEIGHT - 6)
        : belowY;
    scTooltip = { entry, x, y };
  }

  function hideScTooltip() {
    scTooltip = null;
  }
  let pendingPeakThreshold = $state(80);
  let keywordDraft = $state("");
  let peakThresholdTimer: ReturnType<typeof setTimeout> | null = null;
  const DANMU_ITEM_HEIGHT = 28;
  const DANMU_BUFFER = 12;
  const PEAK_THRESHOLD_DEBOUNCE_MS = 300;

  $effect(() => {
    pendingPeakThreshold = peakThreshold;
  });

  let activeRanges = $derived(
    ranges.filter((range) => range.activated !== false),
  );
  let activeDuration = $derived(
    activeRanges.reduce(
      (total, range) => total + Math.max(0, range.end - range.start),
      0,
    ),
  );
  let danmuTypeFilter = $state<"all" | "danmu" | "super_chat">("all");
  let showDanmuTypeFilter = $state(false);
  const DANMU_TYPE_FILTERS = [
    { value: "all", label: "全部" },
    { value: "danmu", label: "弹幕" },
    { value: "super_chat", label: "醒目留言" },
  ] as const;

  function setDanmuTypeFilter(value: "all" | "danmu" | "super_chat") {
    danmuTypeFilter = value;
    showDanmuTypeFilter = false;
    resetDanmuListScroll();
  }

  // Filtering/search shrinks the list; a stale scrollTop would leave the
  // virtualized window past the last entry (blank list).
  function resetDanmuListScroll() {
    danmuScrollTop = 0;
    if (danmuListEl) {
      danmuListEl.scrollTop = 0;
    }
  }

  let typeFilteredDanmu = $derived(
    danmuTypeFilter === "all"
      ? danmuRecords
      : danmuRecords.filter((entry) =>
          danmuTypeFilter === "super_chat"
            ? isSuperChat(entry)
            : !isSuperChat(entry),
        ),
  );
  let filteredDanmu = $derived(
    danmuSearch.trim()
      ? typeFilteredDanmu.filter((entry) =>
          entry.content.toLowerCase().includes(danmuSearch.trim().toLowerCase()),
        )
      : typeFilteredDanmu,
  );
  let keywordMatchedCount = $derived(
    danmuKeywords.length === 0
      ? danmuRecords.length
      : danmuRecords.filter((entry) =>
          danmuKeywords.some((keyword) =>
            entry.content.toLowerCase().includes(keyword.toLowerCase()),
          ),
        ).length,
  );
  let danmuWindow = $derived(
    virtualListWindow(
      danmuScrollTop,
      danmuViewportHeight,
      DANMU_ITEM_HEIGHT,
      filteredDanmu.length,
      DANMU_BUFFER,
    ),
  );

  $effect(() => {
    if (activeTab !== "danmu") return;
    const element = danmuListEl;
    if (!element) return;
    const nextHeight = element.clientHeight;
    if (nextHeight > 0 && Math.abs(nextHeight - danmuViewportHeight) >= 1) {
      danmuViewportHeight = nextHeight;
    }
  });

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

  function normalizeKeyword(value: string) {
    return value.trim().replace(/^,+|,+$/g, "").trim();
  }

  function commitKeywordDraft() {
    const parts = keywordDraft
      .split(/[,，]/)
      .map(normalizeKeyword)
      .filter(Boolean);
    keywordDraft = "";
    if (parts.length === 0) return;

    const next = [...danmuKeywords];
    for (const part of parts) {
      const exists = next.some(
        (keyword) => keyword.toLowerCase() === part.toLowerCase(),
      );
      if (!exists) next.push(part);
    }
    if (next.length !== danmuKeywords.length) {
      onDanmuKeywordsChange?.(next);
    }
  }

  function removeKeyword(index: number) {
    onDanmuKeywordsChange?.(
      danmuKeywords.filter((_, keywordIndex) => keywordIndex !== index),
    );
  }

  function handleKeywordKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === "," || event.key === "，") {
      event.preventDefault();
      commitKeywordDraft();
      return;
    }
    if (
      event.key === "Backspace" &&
      keywordDraft.length === 0 &&
      danmuKeywords.length > 0
    ) {
      event.preventDefault();
      removeKeyword(danmuKeywords.length - 1);
    }
  }

  function schedulePeakThresholdChange(value: number) {
    pendingPeakThreshold = value;
    if (peakThresholdTimer) {
      clearTimeout(peakThresholdTimer);
    }
    peakThresholdTimer = setTimeout(() => {
      onPeakThresholdChange?.(pendingPeakThreshold);
      peakThresholdTimer = null;
    }, PEAK_THRESHOLD_DEBOUNCE_MS);
  }

  onDestroy(() => {
    if (peakThresholdTimer) {
      clearTimeout(peakThresholdTimer);
    }
  });
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
        {#if tab.id === "markers" && markers.length > 0}
          <span>{markers.length}</span>
        {/if}
      </button>
    {/each}
  </div>

  <div
    class="inspector-content"
    class:panel-fill={activeTab === "danmu"}
  >
    {#if activeTab === "ranges"}
      <section class="range-summary">
        <div class="summary-meta">
          <span>已选内容</span>
          <span class="summary-duration">{formatTime(activeDuration)}</span>
          {#if ranges.length > 0}
            <span class="summary-count">{activeRanges.length}/{ranges.length}</span>
          {/if}
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
          <span class="check-mark" aria-hidden="true">
            {#if ranges.length > 0 &&
              ranges.every((range) => range.activated !== false)}
              <Check size={10} strokeWidth={2.5} />
            {/if}
          </span>
          <span>全选</span>
        </label>
      </section>

      <section class="range-list" aria-label="选区列表">
        {#each ranges as range, index}
          <article
            class="range-card"
            class:selected={selectedRangeIndex === index}
            class:inactive={range.activated === false}
          >
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
              <span class="check-mark" aria-hidden="true">
                {#if range.activated !== false}
                  <Check size={10} strokeWidth={2.5} />
                {/if}
              </span>
            </label>
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
        <label class="keyword-field">
          <span>关键词过滤</span>
          <div class="keyword-editor">
            {#each danmuKeywords as keyword, index}
              <span class="keyword-chip">
                {keyword}
                <button
                  type="button"
                  aria-label={`删除关键词 ${keyword}`}
                  onclick={() => removeKeyword(index)}
                >
                  ×
                </button>
              </span>
            {/each}
            <input
              bind:value={keywordDraft}
              placeholder={danmuKeywords.length === 0
                ? "输入关键词后回车，留空则使用全部弹幕"
                : "继续添加…"}
              onkeydown={handleKeywordKeydown}
              onblur={commitKeywordDraft}
            />
          </div>
        </label>
        <label class="threshold">
          <span>峰值阈值 {pendingPeakThreshold}%</span>
          <input
            type="range"
            min="50"
            max="100"
            value={pendingPeakThreshold}
            oninput={(event) =>
              schedulePeakThresholdChange(
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
              {#if danmuRecords.length === 0}
                暂无弹幕数据
              {:else if danmuKeywords.length > 0 && keywordMatchedCount === 0}
                当前关键词下无匹配弹幕
              {:else}
                当前阈值下未发现峰值
              {/if}
            </p>
          {/each}
        </div>
      </section>
    {:else if activeTab === "danmu"}
      <section class="danmu-panel">
        <div
          class="danmu-toolbar"
          use:clickOutside={() => (showDanmuTypeFilter = false)}
        >
          <label class="search-box">
            <Search size={13} />
            <input
              bind:value={danmuSearch}
              oninput={resetDanmuListScroll}
              placeholder="搜索弹幕 / SC 内容"
            />
          </label>
          <div class="type-filter">
            <button
              type="button"
              class="filter-button"
              class:active={danmuTypeFilter !== "all"}
              title="按类型筛选"
              onclick={() => (showDanmuTypeFilter = !showDanmuTypeFilter)}
            >
              <Filter size={13} />
            </button>
            {#if showDanmuTypeFilter}
              <div class="filter-dropdown">
                {#each DANMU_TYPE_FILTERS as option (option.value)}
                  <label class="filter-option">
                    <input
                      type="radio"
                      name="danmu-type-filter"
                      checked={danmuTypeFilter === option.value}
                      onchange={() => setDanmuTypeFilter(option.value)}
                    />
                    <span>{option.label}</span>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        </div>
        <div class="result-count">
          共 {danmuRecords.length} 条 · 显示 {filteredDanmu.length} 条
        </div>
        <div
          class="danmu-list"
          bind:this={danmuListEl}
          onscroll={(event) => {
            const element = event.currentTarget as HTMLElement;
            danmuScrollTop = element.scrollTop;
            danmuViewportHeight = element.clientHeight;
            hideScTooltip();
          }}
        >
          {#if filteredDanmu.length === 0}
            <div class="empty-state">
              <strong>没有匹配的内容</strong>
            </div>
          {:else}
            <div
              class="danmu-list-spacer"
              style:height={`${danmuWindow.totalHeight}px`}
            >
              <div
                class="danmu-list-window"
                style:transform={`translateY(${danmuWindow.offsetY}px)`}
              >
                {#each filteredDanmu.slice(danmuWindow.start, danmuWindow.end) as danmu, index (`${danmu.ts}:${danmu.user_name ?? ""}:${danmu.content}:${danmuWindow.start + index}`)}
                  <button
                    type="button"
                    class="danmu-entry"
                    class:sc-entry={isSuperChat(danmu)}
                    onclick={() => onSeek?.(danmu.ts / 1000 - globalOffset)}
                    onmouseenter={(event) => {
                      if (isSuperChat(danmu)) {
                        showScTooltip(
                          danmu,
                          event.currentTarget as HTMLElement,
                        );
                      }
                    }}
                    onmouseleave={hideScTooltip}
                  >
                    <time>{formatTime(danmu.ts / 1000 - globalOffset)}</time>
                    <span class="danmu-content">
                      {#if isSuperChat(danmu)}
                        <span
                          class="sc-badge"
                          style:background={getScColors(danmu.price ?? 0)
                            .content}
                        >
                          SC {formatScPrice(danmu.price ?? 0)}
                        </span>
                      {/if}
                      {#if danmu.user_name}
                        <b>{danmu.user_name}</b>
                      {/if}
                      <span class="danmu-text">{danmu.content}</span>
                    </span>
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        </div>
        {#if scTooltip}
          {@const colors = getScColors(scTooltip.entry.price ?? 0)}
          <div
            class="sc-tooltip"
            style:left="{scTooltip.x}px"
            style:top="{scTooltip.y}px"
          >
            <div
              class="sc-tooltip-head"
              style:background-color={colors.info}
              style:border-color={colors.content}
            >
              <span class="sc-tooltip-price" style:color={colors.content}>
                {formatScPrice(scTooltip.entry.price ?? 0)}
              </span>
              <span class="sc-tooltip-user">{scTooltip.entry.user_name}</span>
            </div>
            <div
              class="sc-tooltip-body"
              style:background-color={colors.content}
            >
              {scTooltip.entry.content}
            </div>
          </div>
        {/if}
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
              role="button"
              tabindex="0"
              onclick={() => onVideoSelect?.(item.id)}
              onkeydown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onVideoSelect?.(item.id);
                }
              }}
            >
              <div class="clip-preview">
                {#if item.cover}
                  <img src={item.cover} alt="" />
                {:else}
                  <span>暂无封面</span>
                {/if}
              </div>
              <div class="clip-info">
                <span class="clip-name" title={item.name}>{item.name}</span>
                <div
                  class="clip-actions"
                  onclick={(event) => event.stopPropagation()}
                  onkeydown={(event) => event.stopPropagation()}
                >
                  <button
                    type="button"
                    title="打开切片"
                    onclick={() => onOpenVideo?.(item.id)}
                  >
                    <ExternalLink size={13} />
                  </button>
                  {#if !TAURI_ENV}
                    <button
                      type="button"
                      title="下载切片"
                      onclick={() => onDownloadVideo?.(item.id)}
                    >
                      <Download size={13} />
                    </button>
                  {/if}
                  <button
                    type="button"
                    title="删除切片"
                    class="danger"
                    onclick={() => onDeleteVideo?.(item.id)}
                  >
                    <Trash2 size={13} />
                  </button>
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
  .clip-actions button {
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

  .inspector-content.panel-fill {
    display: flex;
    overflow: hidden;
    flex-direction: column;
  }

  .range-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    border-radius: 10px;
    background: #202733;
    padding: 9px 12px;
  }

  .summary-meta {
    display: flex;
    min-width: 0;
    align-items: baseline;
    gap: 8px;
  }

  .range-summary span,
  .range-summary label,
  .section-heading span,
  .result-count {
    color: #8490a3;
    font-size: 10px;
  }

  .summary-duration {
    color: #d5deea;
    font-size: 11px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .summary-count {
    color: #687589;
    font-variant-numeric: tabular-nums;
  }

  .select-all-toggle {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
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
    min-height: 42px;
    align-items: center;
    gap: 2px;
    border: 1px solid transparent;
    border-radius: 10px;
    background: #202733;
    padding-left: 10px;
    transition:
      border-color 150ms ease,
      opacity 150ms ease;
  }

  .range-card.selected {
    border-color: #35a7ff;
    box-shadow: inset 3px 0 #35a7ff;
  }

  .range-card.inactive {
    opacity: 0.48;
  }

  .range-main {
    display: flex;
    height: 40px;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 8px;
    overflow: hidden;
    padding: 0 8px 0 6px;
    text-align: left;
  }

  .range-title {
    width: 14px;
    flex: 0 0 14px;
    overflow: hidden;
    color: #8490a3;
    font-size: 10px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .range-time {
    min-width: 0;
    overflow: hidden;
    flex: 1 1 auto;
    color: #b6c0cf;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .range-duration {
    flex: 0 0 auto;
    color: #687589;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .range-actions {
    display: flex;
    gap: 2px;
    padding-right: 8px;
  }

  .range-actions button,
  .range-toggle {
    display: inline-flex;
    width: 22px;
    height: 22px;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .range-actions button {
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #8290a4;
  }

  .range-actions button:hover {
    background: rgb(255 255 255 / 6%);
    color: #d5deea;
  }

  .clip-actions button {
    width: 27px;
    height: 27px;
    border: 0;
    background: #171c25;
    cursor: pointer;
  }

  .clip-actions button:hover {
    color: #d5deea;
  }

  .check-mark {
    display: inline-flex;
    width: 14px;
    height: 14px;
    align-items: center;
    justify-content: center;
    border: 1px solid #4a5568;
    border-radius: 4px;
    background: transparent;
    color: transparent;
    transition:
      border-color 150ms ease,
      background-color 150ms ease,
      color 150ms ease;
  }

  .select-all-toggle:hover .check-mark,
  .range-toggle:hover .check-mark {
    border-color: #687589;
  }

  .select-all-toggle.checked .check-mark,
  .range-toggle.checked .check-mark {
    border-color: #3d8fd1;
    background: rgb(10 132 255 / 14%);
    color: #7ec8ff;
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

  .keyword-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
    color: #9aa6b8;
    font-size: 10px;
  }

  .keyword-editor {
    display: flex;
    min-height: 36px;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    padding: 6px 8px;
  }

  .keyword-editor:focus-within {
    border-color: #0a84ff;
  }

  .keyword-chip {
    display: inline-flex;
    max-width: 100%;
    align-items: center;
    gap: 4px;
    border: 1px solid #3b4758;
    border-radius: 999px;
    background: #153b5c;
    padding: 2px 4px 2px 8px;
    color: #7ec8ff;
    font-size: 10px;
    line-height: 1.4;
  }

  .keyword-chip button {
    display: inline-flex;
    width: 16px;
    height: 16px;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: #9ecfff;
    font-size: 12px;
    line-height: 1;
  }

  .keyword-chip button:hover {
    background: rgb(255 255 255 / 10%);
    color: #e9eef8;
  }

  .keyword-editor input {
    min-width: 120px;
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: #e9eef8;
    font-size: 10px;
  }

  .keyword-editor input::placeholder {
    color: #718095;
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
    flex: 1;
    flex-direction: column;
  }

  .search-box {
    display: flex;
    height: 30px;
    flex: 0 0 30px;
    align-items: center;
    gap: 6px;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    padding: 0 8px;
    color: #718095;
  }

  .danmu-toolbar {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .danmu-toolbar .search-box {
    min-width: 0;
    flex: 1;
  }

  .type-filter {
    position: relative;
    flex-shrink: 0;
  }

  .filter-button {
    display: inline-flex;
    width: 30px;
    height: 30px;
    align-items: center;
    justify-content: center;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    color: #718095;
    cursor: pointer;
  }

  .filter-button:hover,
  .filter-button.active {
    color: #6eb8ef;
  }

  .filter-button.active {
    border-color: rgb(110 184 239 / 45%);
  }

  .filter-dropdown {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    min-width: 130px;
    border: 1px solid #293442;
    border-radius: 8px;
    background: #202733;
    padding: 4px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 45%);
  }

  .filter-option {
    display: flex;
    align-items: center;
    gap: 8px;
    border-radius: 5px;
    padding: 5px 8px;
    color: #c5cedb;
    font-size: 11px;
    cursor: pointer;
    user-select: none;
  }

  .filter-option:hover {
    background: rgb(255 255 255 / 4%);
  }

  .filter-option input {
    margin: 0;
    accent-color: #4a89dc;
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
    padding: 5px 2px 4px;
    color: #687589;
    font-size: 9px;
  }

  .danmu-list {
    min-height: 0;
    flex: 1;
    overflow-y: auto;
    overflow-anchor: none;
    overscroll-behavior: contain;
    border-top: 1px solid #242b36;
  }

  .danmu-list-spacer {
    position: relative;
    width: 100%;
  }

  .danmu-list-window {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
  }

  .danmu-entry {
    display: flex;
    box-sizing: border-box;
    width: 100%;
    height: 28px;
    min-height: 28px;
    max-height: 28px;
    flex-shrink: 0;
    align-items: center;
    gap: 8px;
    border: 0;
    border-bottom: 1px solid #1c222c;
    border-radius: 0;
    background: transparent;
    padding: 0 4px;
    text-align: left;
  }

  .danmu-entry:hover {
    background: rgb(255 255 255 / 3.5%);
  }

  .danmu-entry .danmu-content {
    display: flex;
    min-width: 0;
    flex: 1;
    align-items: center;
    overflow: hidden;
    color: #c5cedb;
    font-size: 11px;
    line-height: 1.2;
  }

  .danmu-content b {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    margin-right: 5px;
    color: #6eb8ef;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .danmu-content .danmu-text {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .danmu-entry time {
    flex: 0 0 58px;
    color: #5f6b7d;
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .sc-badge {
    display: inline-block;
    box-sizing: border-box;
    flex: 0 0 auto;
    margin-right: 6px;
    border-radius: 3px;
    padding: 0 5px;
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    line-height: 14px;
  }

  .sc-entry .danmu-content {
    color: #eef4fd;
  }

  .sc-tooltip {
    position: fixed;
    z-index: 60;
    width: 260px;
    border-radius: 8px;
    overflow: hidden;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.45);
    pointer-events: none;
  }

  .sc-tooltip-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border: 1px solid;
    border-bottom: 0;
    border-radius: 8px 8px 0 0;
    padding: 5px 10px;
    font-size: 12px;
  }

  .sc-tooltip-price {
    flex-shrink: 0;
    font-weight: 700;
  }

  .sc-tooltip-user {
    min-width: 0;
    overflow: hidden;
    color: #333;
    font-weight: 600;
    opacity: 0.78;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sc-tooltip-body {
    padding: 6px 10px;
    color: #fff;
    font-size: 12px;
    line-height: 1.5;
    word-break: break-word;
    white-space: pre-wrap;
  }

  .clip-card {
    display: grid;
    overflow: hidden;
    grid-template-columns: 94px minmax(0, 1fr);
    border: 1px solid #293442;
    border-radius: 9px;
    background: #202733;
    cursor: pointer;
  }

  .clip-card:hover {
    border-color: #3a4658;
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
    pointer-events: none;
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
    gap: 8px;
    padding: 8px;
  }

  .clip-name {
    overflow: hidden;
    color: #dce5f3;
    font-size: 10px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .clip-actions {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
    opacity: 0;
    pointer-events: none;
    transition: opacity 120ms ease;
  }

  .clip-card:hover .clip-actions,
  .clip-card.selected .clip-actions,
  .clip-card:focus-within .clip-actions {
    opacity: 1;
    pointer-events: auto;
  }

  .clip-actions button.danger,
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

  button:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }

  input:focus,
  input:focus-visible {
    outline: none;
  }

  .keyword-chip button:focus-visible {
    outline: none;
  }
</style>
