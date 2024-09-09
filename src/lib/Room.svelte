<script lang="ts">
  import { invoke } from "@tauri-apps/api";
  import {
    Badge,
    SpeedDial,
    SpeedDialButton,
    Table,
    TableBody,
    TableBodyCell,
    TableBodyRow,
    TableHead,
    TableHeadCell,
  } from "flowbite-svelte";
  import { DownloadSolid, FileCopySolid, PrinterSolid, ShareNodesSolid } from "flowbite-svelte-icons";
  export let room_count = 0;
  interface RoomSummary {
    count: number;
    rooms: {
      room_id: number;
      room_title: string;
      room_cover: string;
      room_keyframe: string;
      user_id: number;
      user_name: string;
      user_sign: string;
      user_avatar: string;
      live_status: boolean;
      total_length: number;
    }[];
  }
  let summary: RoomSummary = {
    count: 0,
    rooms: [],
  };
  async function update_summary() {
    let _summary: RoomSummary = await invoke("get_summary");
    room_count = _summary.count;
    _summary.rooms = await Promise.all(
      _summary.rooms.map(async (room) => {
        return room;
      })
    );
    summary = _summary;
  }
  update_summary();
  setInterval(update_summary, 1000);
  function format_time(time: number) {
    let hours = Math.floor(time / 3600);
    let minutes = Math.floor((time % 3600) / 60);
    let seconds = Math.floor(time % 60);
    return `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }
</script>

<Table hoverable={true}>
  <TableHead>
    <TableHeadCell>房间号</TableHeadCell>
    <TableHeadCell>标题</TableHeadCell>
    <TableHeadCell>账号</TableHeadCell>
    <TableHeadCell>状态</TableHeadCell>
    <TableHeadCell>缓存时长</TableHeadCell>
    <TableHeadCell>
      <span class="sr-only">Edit</span>
    </TableHeadCell>
  </TableHead>
  <TableBody tableBodyClass="divide-y">
    {#each summary.rooms as room}
      <TableBodyRow>
        <TableBodyCell>{room.room_id}</TableBodyCell>
        <TableBodyCell>{room.room_title}</TableBodyCell>
        <TableBodyCell>{room.user_name}</TableBodyCell>
        <TableBodyCell>
            {#if room.live_status}
                <Badge color="green">直播中</Badge>
            {:else}
                <Badge color="dark">未直播</Badge>
            {/if}
        </TableBodyCell>
        <TableBodyCell>{format_time(room.total_length)}</TableBodyCell>
        <TableBodyCell>
          <a
            href={"#"}
            on:click={async (e) => {
              e.preventDefault();
              await invoke("open_live", { roomId: room.room_id });
            }}
            class="font-medium text-primary-600 hover:underline dark:text-primary-500"
            >切片</a
          >
        </TableBodyCell>
      </TableBodyRow>
    {/each}
  </TableBody>
</Table>

<SpeedDial defaultClass="fixed end-6 bottom-6">
  <SpeedDialButton name="Share">
    <ShareNodesSolid class="w-6 h-6" />
  </SpeedDialButton>
  <SpeedDialButton name="Print">
    <PrinterSolid class="w-6 h-6" />
  </SpeedDialButton>
  <SpeedDialButton name="Download">
    <DownloadSolid class="w-6 h-6" />
  </SpeedDialButton>
  <SpeedDialButton name="Copy">
    <FileCopySolid class="w-6 h-6" />
  </SpeedDialButton>
</SpeedDial>
