<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { fetch } from "@tauri-apps/plugin-http";
    import { Card, List, Li, Tooltip } from "flowbite-svelte";
    import { GithubSolid, GlobeSolid } from "flowbite-svelte-icons";
    import Image from "./Image.svelte";
    import type { RecorderList } from "./interface";
    import type { RecordItem } from "./db";
    const INTERVAL = 5000;

    let summary: RecorderList = {
        count: 0,
        recorders: [],
    };

    let total = 0;
    let online = 0;
    let disk_usage = 0;

    async function update_summary() {
        summary = (await invoke("get_recorder_list")) as RecorderList;
        total = summary.count;
        online = summary.recorders.filter((r) => r.live_status).length;
        // each recorder get archive size
        disk_usage = 0;
        for (const recorder of summary.recorders) {
            disk_usage += await get_disk_usage(recorder.room_id);
        }
    }
    update_summary();
    setInterval(update_summary, INTERVAL);

    async function get_disk_usage(room_id: number) {
        const archives = (await invoke("get_archives", {
            roomId: room_id,
        })) as RecordItem[];
        for (const archive of archives) {
            disk_usage += archive.size;
        }
        return disk_usage;
    }

    function format_size(size: number) {
        if (size < 1024) {
            return `${size} B`;
        } else if (size < 1024 * 1024) {
            return `${(size / 1024).toFixed(2)} KiB`;
        } else if (size < 1024 * 1024 * 1024) {
            return `${(size / 1024 / 1024).toFixed(2)} MiB`;
        } else {
            return `${(size / 1024 / 1024 / 1024).toFixed(2)} GiB`;
        }
    }

    interface Sponser {
        name: string;
        avatar: string;
    }
    let sponsers: Sponser[] = [];
    async function get_sponsers() {
        const response = await fetch(
            "https://afdian.com/api/creator/get-sponsors?user_id=bbb3f596df9c11ea922752540025c377&type=new&page=1",
        );
        const data = await response.json();
        console.log(data);
        if (data.ec == 200) {
            sponsers = data.data.list.slice(0, 10);
        }
    }
    get_sponsers();
</script>

<div class="grid grid-cols-2 gap-4 p-8 pt-12">
    <Card class="!max-w-none">
        <h5
            class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
        >
            支持该项目的开发
        </h5>
        <List tag="ul" class="space-y-1 text-gray-500">
            <Li
                >反馈 BUG 或提供建议：<a
                    href="https://github.com/Xinrea/bili-shadowreplay"
                    target="_blank"><GithubSolid class="inline" />GitHub</a
                ></Li
            >
            <Li
                >赞助：<a href="https://afdian.com/a/Xinrea" target="_blank"
                    ><GlobeSolid class="inline" />爱发电</a
                ></Li
            >
        </List>
        <div class="mt-4 flex flex-row items-center">
            <span>感谢</span>
            {#each sponsers as sp}
                <Image iclass="rounded-full w-8" src={sp.avatar} />
                <Tooltip>{sp.name}</Tooltip>
            {/each}
            <span>等的赞助</span>
        </div>
    </Card>

    <Card class="!max-w-none">
        <h5
            class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
        >
            直播间总览
        </h5>
        <p class="font-normal text-gray-700 dark:text-gray-400 leading-tight">
            目前共有 {total} 个直播间，其中 {online} 个正在直播，{total -
                online} 个未直播；共占用磁盘空间 {format_size(disk_usage)}。
        </p>
    </Card>
</div>
