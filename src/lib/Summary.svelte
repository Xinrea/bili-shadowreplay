<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { Card } from "flowbite-svelte";
  const INTERVAL = 1000;
  interface SummaryInfo {
    room: {
      total: number;
      online: number;
      offline: number;
    };
  }
  let summary_info = {
    room: {
      total: 0,
      online: 0,
      offline: 0,
    },
  };
  refresh();
  setInterval(refresh, INTERVAL);
  function refresh() {
    // invoke("get_summary_info").then((res) => {
    //     summary_info = res as SummaryInfo;
    // });
  }
</script>

<div class="grid grid-cols-2 gap-4 p-8 pt-12">
  <Card class="!max-w-none">
    <h5
      class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
    >
      该项目需要你的支持
    </h5>
    <p class="font-normal text-gray-700 dark:text-gray-400 leading-tight">
      如果你喜欢这个项目，可以通过以下方式来支持我：
    </p>
    <p>- 反馈 BUG 以及提出建议</p>
    <p>- 分享给你的朋友</p>
    <p>- 赞助</p>
  </Card>

  <Card class="!max-w-none">
    <h5
      class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
    >
      直播间总览
    </h5>
    <p class="font-normal text-gray-700 dark:text-gray-400 leading-tight">
      目前共有 {summary_info.room.total} 个直播间，其中 {summary_info.room
        .online} 个正在直播，{summary_info.room.offline} 个未直播。
    </p>
  </Card>
  <Card class="!max-w-none">444</Card>
  <Card class="!max-w-none">444</Card>
</div>
