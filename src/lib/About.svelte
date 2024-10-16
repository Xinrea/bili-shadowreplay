<script type="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { Card } from "flowbite-svelte";
  const appWindow = getCurrentWebviewWindow();
  let version = "";
  getVersion().then((v) => {
    version = v;
    appWindow.setTitle(`BiliBili ShadowReplay - v${version}`);
    console.log(version);
  });
  let latest_version = "";
  // get lastest version from github release api
  fetch("https://api.github.com/repos/Xinrea/bili-shadowreplay/releases/latest")
    .then((response) => response.json())
    .then((data) => {
      latest_version = data.tag_name;
    });
</script>

<div class="p-8 pt-12 h-full overflow-auto">
  <Card size="lg">
    <h1 class="text-2xl font-bold">关于</h1>
    <p>
      BiliBili ShadowReplay 是一个用于实时查看和剪辑 B
      站直播流，并生成视频投稿的工具。
    </p>
    <p class="mt-4">
      项目地址: <a href="https://github.com/Xinrea/bili-shadowreplay"
        >https://github.com/Xinrea/bili-shadowreplay</a
      >
    </p>
    <p>作者: Xinrea</p>
    <p>
      当前版本: v{version}
    </p>
    <p>
      最新版本: {latest_version}
    </p>
  </Card>
</div>
