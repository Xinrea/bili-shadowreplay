<script lang="ts">
  import type { StorageConfig } from "../interface";
  import { storageBackend, storageBackendEntries } from "../recording-storage";

  let {
    storage, value = $bindable<string | null>(null), id, disabled = false,
    inherit = true,
  }: {
    storage: StorageConfig;
    value?: string | null;
    id: string;
    disabled?: boolean;
    inherit?: boolean;
  } = $props();
  const selected = $derived(value === null ? storage.default_backend : value);
  const backend = $derived(storageBackend(storage, selected));
</script>

<select {id} class="ui-input w-full min-w-0" bind:value {disabled}>
  {#if inherit}
    <option value={null}>跟随应用默认（{storage.default_backend || "未配置"}）</option>
  {/if}
  {#if value !== null && !storageBackend(storage, value)}
    <option value={value} disabled>未知存储：{value || "未配置"}</option>
  {/if}
  {#each storageBackendEntries(storage) as [backendId, entry] (backendId)}
    <option value={backendId} disabled={entry.type !== "local"}>{backendId}{entry.type !== "local" ? "（暂不支持）" : ""}</option>
  {/each}
</select>
{#if backend}
  <p class="ui-hint mt-1 select-text break-all">{backend.root}</p>
{:else}
  <p class="mt-1 text-[12px] text-danger-fg" role="alert">存储 {selected || "（未配置）"} 不存在，请在设置中检查。不会自动改用其他存储。</p>
{/if}
