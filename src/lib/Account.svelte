<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import {
    Button,
    Card,
    Table,
    TableHead,
    TableHeadCell,
    TableBody,
    TableBodyRow,
    TableBodyCell,
    Modal,
    ButtonGroup,
  } from "flowbite-svelte";
  import Image from "./Image.svelte";
  import QRCode from "qrcode";
  import type { AccountItem, AccountInfo } from "./db";
  import { PlusOutline } from "flowbite-svelte-icons";

  let account_info: AccountInfo = {
    primary_uid: 0,
    accounts: [],
  };

  async function update_accounts() {
    account_info = await invoke("get_accounts");
  }

  update_accounts();

  let addModal = false;
  let oauth_key = "";
  let check_interval = null;

  async function handle_qr() {
    if (check_interval) {
      clearInterval(check_interval);
    }
    let qr_info: { url: string; oauthKey: string } = await invoke("get_qr");
    oauth_key = qr_info.oauthKey;
    const canvas = document.getElementById("qr");
    QRCode.toCanvas(canvas, qr_info.url, function (error) {
      if (error) {
        console.log(error);
        return;
      }
      canvas.style.display = "block";
      check_interval = setInterval(check_qr, 2000);
    });
  }

  async function check_qr() {
    let qr_status: { code: number; cookies: string } = await invoke(
      "get_qr_status",
      { qrcodeKey: oauth_key },
    );
    if (qr_status.code == 0) {
      clearInterval(check_interval);
      await invoke("add_account", { cookies: qr_status.cookies });
      await update_accounts();
      addModal = false;
    }
  }
</script>

<div class="p-8 pt-12 h-full overflow-auto">
  <Table hoverable={true} divClass="relative max-h-full" shadow>
    <TableHead>
      <TableHeadCell>UID</TableHeadCell>
      <TableHeadCell>头像</TableHeadCell>
      <TableHeadCell>用户名</TableHeadCell>
      <TableHeadCell>状态</TableHeadCell>
      <TableHeadCell>添加时间</TableHeadCell>
      <TableHeadCell>操作</TableHeadCell>
    </TableHead>
    <TableBody tableBodyClass="divide-y">
      {#each account_info.accounts as account}
        <TableBodyRow>
          <TableBodyCell>{account.uid}</TableBodyCell>
          <TableBodyCell
            ><Image
              iclass="rounded-full w-12"
              src={account.avatar}
            /></TableBodyCell
          >
          <TableBodyCell>{account.name}</TableBodyCell>
          <TableBodyCell
            >{account.uid == account_info.primary_uid
              ? "主账号"
              : "普通账号"}</TableBodyCell
          >
          <TableBodyCell
            >{new Date(account.created_at).toLocaleString()}</TableBodyCell
          >
          <TableBodyCell>
            <ButtonGroup>
              <Button
                on:click={async () => {
                  await invoke("remove_account", { uid: account.uid });
                  await update_accounts();
                }}>注销</Button
              >
              {#if account.uid != account_info.primary_uid}
                <Button
                  on:click={async () => {
                    await invoke("set_primary", { uid: account.uid });
                    await update_accounts();
                  }}>设置为主账号</Button
                >
              {/if}
            </ButtonGroup></TableBodyCell
          >
        </TableBodyRow>
      {/each}
    </TableBody>
  </Table>
</div>

<div class="fixed end-4 bottom-4">
  <Button
    pill={true}
    class="!p-2"
    on:click={() => {
      addModal = true;
      requestAnimationFrame(handle_qr);
    }}><PlusOutline class="w-8 h-8" /></Button
  >
</div>

<Modal bind:open={addModal} autoclose>
  <canvas id="qr" />
</Modal>
