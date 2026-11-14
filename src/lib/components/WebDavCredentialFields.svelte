<script lang="ts">
  let { idPrefix, username = $bindable(""), password = $bindable("") }: {
    idPrefix: string; username?: string; password?: string;
  } = $props();
</script>

<div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
  <div>
    <label for={`${idPrefix}-username`} class="ui-label mb-1 block">用户名</label>
    <input id={`${idPrefix}-username`} class="ui-input w-full" bind:value={username} autocomplete="off" spellcheck={false} required />
  </div>
  <div>
    <label for={`${idPrefix}-password`} class="ui-label mb-1 block">密码 / 应用密码</label>
    <input id={`${idPrefix}-password`} type="password" class="ui-input w-full" bind:value={password} autocomplete="new-password" required />
  </div>
</div>
<p class="ui-hint">凭证单独保存，重新打开时不会显示已保存的密码。</p>
