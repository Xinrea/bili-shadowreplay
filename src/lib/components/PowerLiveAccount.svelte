<script lang="ts">
  import { onMount } from "svelte";
  import { ExternalLink, RefreshCw, UserRound } from "lucide-svelte";
  import { invoke, open, TAURI_ENV } from "../invoker";
  import { createStateRefresh } from "../state-refresh";

  let { active = true }: { active?: boolean } = $props();
  type Status = { logged_in: boolean; name: string; subject: string; pending: boolean; error: string | null };
  type AccountInfo = { subject: string; name: string; credit: number };
  const creditFormat = new Intl.NumberFormat("zh-CN");
  const creditRefreshMs = 60_000;
  let status: Status = $state({ logged_in: false, name: "", subject: "", pending: false, error: null });
  let account: AccountInfo | null = $state(null);
  let creditLoading = $state(false);
  let creditError = $state("");
  let creditUpdatedAt = $state("");
  let busy = $state(false);
  let error = $state("");
  let statusError = $state("");
  let logoutFailed = $state(false);
  let loading = $state(true);
  let mounted = false;
  let lastCreditAttempt = 0;
  const displayName = $derived(account?.name || status.name || "PowerLive 用户");

  async function refreshCredit() {
    if (!mounted || busy || creditLoading || !status.logged_in || status.pending) return;
    const subject = status.subject;
    lastCreditAttempt = Date.now();
    creditLoading = true;
    try {
      const result = await invoke<AccountInfo>("get_powerlive_account");
      if (mounted && !busy && status.logged_in && status.subject === subject) {
        if (result.subject !== subject || !Number.isFinite(result.credit)) throw new Error("PowerLive 额度信息无效，请重新登录");
        account = result;
        creditError = "";
        creditUpdatedAt = new Date().toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
      }
    } catch (e) {
      if (mounted && !busy && status.subject === subject) {
        creditError = String(e);
        await stateRefresh.run();
        if (mounted && !status.logged_in) error = String(e);
      }
    } finally {
      if (mounted) {
        creditLoading = false;
        if (status.subject === subject) lastCreditAttempt = Date.now();
      }
    }
  }

  async function refreshStatus() {
    try {
      const result = await invoke<Status>("get_powerlive_status");
      if (!mounted) return;
      if (result.subject !== status.subject || result.logged_in !== status.logged_in) {
        account = null;
        creditUpdatedAt = "";
        creditError = "";
        lastCreditAttempt = 0;
        logoutFailed = false;
      }
      status = result;
      loading = false;
      statusError = "";
      if (active && document.visibilityState !== "hidden" && Date.now() - lastCreditAttempt >= creditRefreshMs) {
        void refreshCredit();
      }
    } catch (e) {
      if (mounted) { statusError = String(e); loading = false; }
    }
  }
  const stateRefresh = createStateRefresh(refreshStatus);

  function refreshAccount() {
    lastCreditAttempt = 0;
    return stateRefresh.run();
  }
  async function login() {
    busy = true; error = "";
    try {
      const url = await invoke<string>("begin_powerlive_login");
      try { await open(url); }
      catch (e) { await invoke("cancel_powerlive_login"); throw e; }
    } catch (e) { error = String(e); }
    finally { busy = false; await refreshAccount(); }
  }
  async function cancel() {
    busy = true; error = "";
    try { await invoke("cancel_powerlive_login"); }
    catch (e) { error = String(e); }
    finally { busy = false; await refreshAccount(); }
  }
  async function logout(localOnly = false) {
    busy = true; error = ""; logoutFailed = false;
    try { await invoke("logout_powerlive", { localOnly }); }
    catch (e) { error = String(e); logoutFailed = true; }
    finally { busy = false; await refreshAccount(); }
  }
  async function openDashboard() {
    error = "";
    try { await open("https://powerlive.io/dashboard"); }
    catch (e) { error = String(e); }
  }

  $effect(() => {
    if (TAURI_ENV && active) void refreshAccount();
  });
  onMount(() => {
    mounted = true;
    if (!TAURI_ENV) { loading = false; return; }
    void stateRefresh.run();
    const resync = () => {
      if (active && !busy && document.visibilityState !== "hidden") void refreshAccount();
    };
    const timer = setInterval(() => {
      if (busy || document.visibilityState === "hidden") return;
      if (status.pending || (active && status.logged_in && !creditLoading && Date.now() - lastCreditAttempt >= creditRefreshMs)) {
        void stateRefresh.run();
      }
    }, 1000);
    window.addEventListener("focus", resync);
    window.addEventListener("online", resync);
    document.addEventListener("visibilitychange", resync);
    return () => {
      mounted = false;
      clearInterval(timer);
      stateRefresh.stop();
      window.removeEventListener("focus", resync);
      window.removeEventListener("online", resync);
      document.removeEventListener("visibilitychange", resync);
    };
  });
</script>

<section class="powerlive-account" aria-labelledby="powerlive-account-title">
  <header class="account-header">
    <div class="section-heading">
      <h3 id="powerlive-account-title">PowerLive 账号</h3>
    </div>
    {#if TAURI_ENV}
      <div class="account-actions">
        {#if status.pending}
          <button type="button" class="secondary-button" disabled={busy} onclick={cancel}>取消登录</button>
        {:else if status.logged_in}
          <button type="button" class="secondary-button" disabled={busy} onclick={openDashboard}>
            PowerLive 控制台
            <ExternalLink size={14} aria-hidden="true" />
          </button>
          <button type="button" class="logout-button" disabled={busy} onclick={() => logout()}>退出登录</button>
        {:else}
          <button type="button" class="primary-button" disabled={busy || loading} onclick={login}>{busy ? "处理中…" : "登录 PowerLive"}</button>
        {/if}
      </div>
    {/if}
  </header>

  {#if status.logged_in && !status.pending}
    <div class="account-summary">
      <div class="identity">
        <img class="powerlive-logo" src="/powerlive.svg" alt="PowerLive" width="36" height="36" />
        <div class="identity-copy">
          <div class="identity-heading">
            <p class="account-name">{displayName}</p>
            <span class="connection-badge"><span aria-hidden="true"></span>已连接</span>
          </div>
        </div>
      </div>
      <div class="credit-panel" aria-busy={creditLoading}>
        <div class="credit-heading">
          <p>剩余额度</p>
          <button type="button" class="refresh-button" class:refreshing={creditLoading} disabled={busy || creditLoading} onclick={refreshAccount} aria-label={creditLoading ? "正在刷新额度" : "刷新额度"} title={creditLoading ? "正在刷新额度" : "刷新额度"}>
            <RefreshCw size={14} aria-hidden="true" />
          </button>
        </div>
        <p class="credit-value" class:credit-unavailable={!account} aria-live="polite">{account ? creditFormat.format(account.credit) : creditLoading ? "加载中…" : "暂不可用"}</p>
        <p class="credit-meta">每分钟自动刷新{creditUpdatedAt ? ` · 更新于 ${creditUpdatedAt}` : ""}</p>
        {#if creditError}
          <p class="credit-error" role="status">{creditError}{account ? "（当前显示上次更新额度）" : ""}</p>
        {/if}
      </div>
    </div>
  {:else}
    <div class="empty-state" aria-live="polite">
      <div class="empty-icon" aria-hidden="true"><UserRound size={20} /></div>
      <p>
        {#if !TAURI_ENV}请在桌面版设置中登录 PowerLive 账号。
        {:else if loading}正在读取登录状态…
        {:else if status.pending}请在浏览器中完成登录，完成后将自动返回软件。
        {:else}连接 PowerLive 账号，使用账号额度生成字幕。{/if}
      </p>
    </div>
  {/if}
  {#if error || statusError || status.error}<p class="account-error" role="alert">{error || statusError || status.error}</p>{/if}
  {#if logoutFailed && status.logged_in}
    <div class="local-logout">
      <p>仅清除本机凭据，不撤销远端授权。</p>
      <button type="button" class="secondary-button" disabled={busy} onclick={() => logout(true)}>仅清除本地登录</button>
    </div>
  {/if}
</section>

<style>
  .powerlive-account {
    container-type: inline-size;
    padding: 16px;
    color: #111827;
    --muted: #6b7280;
    --border: #e5e7eb;
    --soft: #f8fafc;
    --surface: #fff;
    --hover: #f3f4f6;
  }
  .account-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
  .section-heading h3 { margin: 0; font-size: 14px; font-weight: 600; line-height: 1.5; }
  .account-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  button { display: inline-flex; align-items: center; justify-content: center; gap: 8px; flex-shrink: 0; min-height: 34px; padding: 7px 12px; border: 1px solid transparent; border-radius: 7px; font: inherit; font-size: 12px; font-weight: 500; line-height: 1.5; cursor: pointer; transition: background-color 150ms, color 150ms; }
  button:focus-visible { outline: 2px solid #2563eb; outline-offset: 3px; }
  button:disabled { opacity: 0.5; cursor: default; }
  .secondary-button { border-color: var(--border); color: inherit; background: var(--surface); }
  .secondary-button:hover:enabled { background: var(--hover); }
  .logout-button { color: var(--muted); background: transparent; }
  .logout-button:hover:enabled { color: #dc2626; background: #fef2f2; }
  .primary-button { color: #fff; background: #2563eb; }
  .primary-button:hover:enabled { background: #1d4ed8; }
  .account-summary { display: grid; grid-template-columns: minmax(0, 1fr) minmax(250px, 32%); align-items: center; border: 1px solid var(--border); border-radius: 10px; background: var(--soft); }
  .identity { display: flex; align-items: center; gap: 12px; min-width: 0; padding: 16px; }
  .powerlive-logo { display: block; flex-shrink: 0; width: 36px; height: 36px; }
  .identity-copy { min-width: 0; }
  .identity-heading { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 10px; }
  .account-name { margin: 0; font-size: 15px; font-weight: 600; line-height: 1.5; overflow-wrap: anywhere; }
  .connection-badge { display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0; padding: 2px 7px; border-radius: 5px; color: #15803d; background: #eaf5ed; font-size: 10px; font-weight: 500; line-height: 1.5; }
  .connection-badge span { width: 4px; height: 4px; border-radius: 50%; background: currentColor; }
  .credit-panel { min-width: 0; margin-block: 12px; padding: 0 16px; border-left: 1px solid var(--border); }
  .credit-heading { display: flex; align-items: center; gap: 6px; min-height: 24px; }
  .credit-heading p { margin: 0; color: var(--muted); font-size: 12px; line-height: 1.5; }
  .refresh-button { min-height: 24px; width: 24px; padding: 4px; color: var(--muted); background: transparent; border-radius: 5px; }
  .refresh-button:hover:enabled { color: #2563eb; background: var(--border); }
  .refreshing :global(svg) { animation: refresh-spin 1s linear infinite; }
  .credit-value { margin: 0 0 2px; font-size: 24px; font-weight: 600; line-height: 1.25; letter-spacing: -0.6px; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
  .credit-unavailable { margin-block: 7px; color: var(--muted); font-size: 18px; letter-spacing: 0; }
  .credit-meta { margin: 0; color: var(--muted); font-size: 11px; line-height: 1.6; }
  .empty-state { display: flex; align-items: center; gap: 10px; padding: 12px; border: 1px solid var(--border); border-radius: 10px; color: var(--muted); background: var(--soft); }
  .empty-icon { display: grid; place-items: center; flex-shrink: 0; width: 32px; height: 32px; border: 1px solid var(--border); border-radius: 50%; background: var(--surface); }
  .empty-state p { margin: 0; font-size: 13px; line-height: 1.7; }
  .credit-error, .account-error { margin: 8px 0 0; color: #dc2626; font-size: 12px; line-height: 1.7; overflow-wrap: anywhere; }
  .account-error { margin-top: 12px; }
  .local-logout { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 12px; margin-top: 8px; }
  .local-logout p { margin: 0; color: var(--muted); font-size: 12px; line-height: 1.6; }
  :global(.dark) .powerlive-account { color: #f3f4f6; --muted: #b1b6bf; --border: #55565b; --soft: #333438; --surface: #3c3c3e; --hover: #48494d; }
  :global(.dark) .connection-badge { color: #9bddb2; background: #284735; }
  :global(.dark) .logout-button:hover:enabled { color: #fca5a5; background: #512d32; }
  :global(.dark) .credit-error, :global(.dark) .account-error { color: #fca5a5; }
  @container (max-width: 600px) {
    .account-header { flex-wrap: wrap; }
    .account-summary { grid-template-columns: minmax(0, 1fr); }
    .identity { padding: 12px 16px; }
    .credit-panel { margin: 0 16px; padding: 12px 0; border-left: 0; border-top: 1px solid var(--border); }
  }
  @container (max-width: 360px) {
    .account-actions { width: 100%; }
    .identity { gap: 10px; }
  }
  @keyframes refresh-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    button { transition: none; }
    .refreshing :global(svg) { animation: none; }
  }
</style>
