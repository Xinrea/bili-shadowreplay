<script lang="ts">
  let {
    idPrefix,
    accessKeyId = $bindable(""),
    secretAccessKey = $bindable(""),
    sessionToken = $bindable(""),
  }: {
    idPrefix: string;
    accessKeyId?: string;
    secretAccessKey?: string;
    sessionToken?: string;
  } = $props();
</script>

<div class="space-y-3">
  <div>
    <label for={`${idPrefix}-access-key`} class="ui-label mb-1 block">Access Key ID</label>
    <input id={`${idPrefix}-access-key`} class="ui-input w-full" bind:value={accessKeyId} autocomplete="off" spellcheck={false} required />
  </div>
  <div>
    <label for={`${idPrefix}-secret-key`} class="ui-label mb-1 block">Secret Access Key</label>
    <input id={`${idPrefix}-secret-key`} class="ui-input w-full" type="password" bind:value={secretAccessKey} autocomplete="new-password" required />
  </div>
  <div>
    <label for={`${idPrefix}-session-token`} class="ui-label mb-1 block">Session Token（可选）</label>
    <input id={`${idPrefix}-session-token`} class="ui-input w-full" type="password" bind:value={sessionToken} autocomplete="new-password" placeholder="仅使用临时凭证时填写" />
  </div>
  <p class="ui-hint">凭证保存后不回显。更新时请填写完整的一组凭证；Session Token 留空表示不使用临时令牌。远程访问应用设置时请使用 HTTPS。</p>
</div>
