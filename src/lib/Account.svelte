<script lang="ts">
  import { invoke } from "@tauri-apps/api";
  import { Button } from "flowbite-svelte";
  import Image from "./Image.svelte";
  import QRCode from "qrcode";
  interface AccountInfo {
    login: boolean;
    uid: string;
    name: string;
    sign: string;
    face: string;
  }
  let account_info: AccountInfo = {
    login: false,
    uid: "",
    name: "",
    sign: "",
    face: "",
  };
  invoke("get_accounts").then((data: AccountInfo) => {
    account_info = data;
    console.log(account_info);
  });

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
      await invoke("set_cookies", { cookies: qr_status.cookies });
      account_info = await invoke("get_accounts");
    }
  }
</script>

<div class="flex flex-row items-center">
  {#if account_info.login}
    <Image iclass="w-32 h-32 rounded-full p-4" src={account_info.face} />
    <div class="flex flex-col">
      {account_info.name}
      {account_info.uid}
      <Button
        on:click={async () => {
          await invoke("logout");
          account_info.login = false;
        }}>注销</Button
      >
    </div>
  {:else}
    <canvas id="qr" style="display: none;" />
    <Button on:click={handle_qr}>获取登录二维码</Button>
  {/if}
</div>
