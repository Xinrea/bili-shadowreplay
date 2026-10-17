<script lang="ts" generics="T extends string">
  import type { Snippet } from "svelte";
  import { ChevronRight, PanelRightOpen } from "lucide-svelte";
  import TaskCenterButton from "./TaskCenterButton.svelte";

  // The right-hand panel of preview windows. Place it as the last child of a
  // `position: relative` row; below 1100px it overlays the content instead.

  interface Props {
    title: string;
    tabs: { id: T; label: string; count?: number }[];
    activeTab: T;
    collapsed?: boolean;
    /** Let the active tab manage its own scrolling, e.g. a virtual list. */
    fillContent?: boolean;
    /** Shown at the left of the bottom bar. */
    summary?: { label: string; value: string } | null;
    /** Buttons at the right of the bottom bar. */
    actions?: Snippet;
    children: Snippet;
  }

  let {
    title,
    tabs,
    activeTab = $bindable(),
    collapsed = $bindable(false),
    fillContent = false,
    summary = null,
    actions,
    children,
  }: Props = $props();
</script>

<div class="inspector-shell" class:collapsed>
  {#if collapsed}
    <button
      type="button"
      class="open-inspector"
      title="展开{title}"
      aria-label="展开{title}"
      onclick={() => (collapsed = false)}
    >
      <PanelRightOpen size={17} />
    </button>
  {:else}
    <aside class="inspector" aria-label={title}>
      <div class="inspector-heading">
        <h2>{title}</h2>
        <div class="heading-actions">
          <TaskCenterButton tone="dark" />
          <button
            type="button"
            class="collapse-button"
            title="收起{title}"
            aria-label="收起{title}"
            onclick={() => (collapsed = true)}
          >
            <ChevronRight size={17} />
          </button>
        </div>
      </div>

      <div
        class="tabs"
        role="tablist"
        aria-label={title}
        style:grid-template-columns="repeat({tabs.length}, 1fr)"
      >
        {#each tabs as tab (tab.id)}
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === tab.id}
            class:active={activeTab === tab.id}
            onclick={() => (activeTab = tab.id)}
          >
            {tab.label}
            {#if tab.count}
              <span>{tab.count}</span>
            {/if}
          </button>
        {/each}
      </div>

      <div class="inspector-content" class:panel-fill={fillContent}>
        {@render children()}
      </div>

      {#if summary || actions}
        <footer class="inspector-footer">
          <div class="summary">
            {#if summary}
              <span>{summary.label}</span>
              <strong>{summary.value}</strong>
            {/if}
          </div>
          {#if actions}
            <div class="actions">{@render actions()}</div>
          {/if}
        </footer>
      {/if}
    </aside>
  {/if}
</div>

<style>
  .inspector-shell {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 510;
    width: 340px;
    flex: 0 0 auto;
    overflow: visible;
    box-shadow: -12px 0 32px rgb(0 0 0 / 28%);
    transition: width 200ms ease;
  }

  .inspector-shell.collapsed {
    width: 0;
    box-shadow: none;
  }

  @media (min-width: 1100px) {
    .inspector-shell {
      position: relative;
      box-shadow: none;
    }
  }

  .open-inspector {
    position: absolute;
    top: 12px;
    right: 8px;
    display: inline-flex;
    width: 34px;
    height: 34px;
    align-items: center;
    justify-content: center;
    border: 1px solid #344052;
    border-radius: 9px;
    background: #202733;
    color: #dce5f3;
    box-shadow: 0 6px 18px rgb(0 0 0 / 30%);
  }

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

  .heading-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .collapse-button {
    display: inline-flex;
    width: 28px;
    height: 28px;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    background: #222a36;
    color: #8c98aa;
  }

  .tabs {
    display: grid;
    margin: 0 16px 14px;
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

  .summary {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
  }

  .summary span {
    color: #7f8b9d;
    font-size: 9px;
  }

  .summary strong {
    color: #dce5f3;
    font-size: 10px;
    font-weight: 500;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: 6px;
  }

  button:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }
</style>
