<script lang="ts">
  import { invoke } from "@tauri-apps/api";
  import { message } from "@tauri-apps/api/dialog";
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
    Dropdown,
    DropdownItem,
    Button,
    CheckboxButton,
    ButtonGroup,
    Modal,
    Label,
    Select,
    Checkbox,
    Input,
    Helper,
  } from "flowbite-svelte";
  import {
    ChevronDownOutline,
    PlusOutline,
    ExclamationCircleOutline,
  } from "flowbite-svelte-icons";
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
      current_ts: number;
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
      }),
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

  // modals
  let deleteModal = false;
  let deleteRoom = 0;

  let quickClipModal = false;
  let quickClipRoom = 0;
  let quickClipSelected = 0;
  let quickClipOptions = [
    { value: 10, name: "10 秒" },
    { value: 30, name: "30 秒" },
    { value: 60, name: "60 秒" },
  ];

  let addModal = false;
  let addRoom = "";
  let addValid = false;
  let addErrorMsg = "";

  let archiveModal = false;
  let archiveRoom = null;
  let archives = [];
  async function showArchives(room_id: number) {
    archives = await invoke("get_archives", { roomId: room_id });
    archiveModal = true;
    console.log(archives);
  }
  function format_ts(ts: number) {
    const date = new Date(ts * 1000);
    const year = date.getFullYear();
    const month = String(date.getMonth() + 1).padStart(2, "0"); // Months are zero-based
    const day = String(date.getDate()).padStart(2, "0");
    const hours = String(date.getHours()).padStart(2, "0");
    const minutes = String(date.getMinutes()).padStart(2, "0");
    const seconds = String(date.getSeconds()).padStart(2, "0");

    return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
  }
</script>

<Table hoverable={true} divClass="h-full relative overflow-x-auto">
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
          <Button size="sm" color="dark"
            >操作<ChevronDownOutline
              class="w-6 h-6 ms-2 text-white dark:text-white"
            /></Button
          >
          <Dropdown>
            {#if room.live_status}
              <DropdownItem
                on:click={async () => {
                  await invoke("open_live", {
                    roomId: room.room_id,
                    ts: room.current_ts,
                  });
                }}>打开直播流</DropdownItem
              >
              <DropdownItem
                on:click={() => {
                  quickClipRoom = room.room_id;
                  quickClipSelected = 30;
                  quickClipModal = true;
                }}>快速切片</DropdownItem
              >
            {/if}
            <DropdownItem
              on:click={() => {
                archiveRoom = room;
                showArchives(room.room_id);
              }}>查看历史记录</DropdownItem
            >
            <DropdownItem
              on:click={() => {
                deleteRoom = room.room_id;
                deleteModal = true;
              }}>移除直播间</DropdownItem
            >
          </Dropdown>
        </TableBodyCell>
      </TableBodyRow>
    {/each}
  </TableBody>
</Table>

<div class="fixed end-6 bottom-6">
  <Button
    pill={true}
    class="!p-2"
    on:click={() => {
      addModal = true;
    }}><PlusOutline class="w-8 h-8" /></Button
  >
</div>

<Modal bind:open={deleteModal} size="xs" autoclose>
  <div class="text-center">
    <ExclamationCircleOutline
      class="mx-auto mb-4 text-gray-400 w-12 h-12 dark:text-gray-200"
    />
    <h3 class="mb-5 text-lg font-normal text-gray-500 dark:text-gray-400">
      确定要删除这个直播间吗？
    </h3>
    <Button
      color="red"
      class="me-2"
      on:click={async () => {
        await invoke("remove_recorder", { roomId: deleteRoom });
      }}>确定</Button
    >
    <Button color="alternative">取消</Button>
  </div>
</Modal>

<Modal title="快速切片" bind:open={quickClipModal} size="xs" autoclose>
  <Label>
    选择切片时长
    <Select
      class="mt-2"
      items={quickClipOptions}
      bind:value={quickClipSelected}
    />
  </Label>
  <Checkbox>生成后启动上传流程</Checkbox>
  <Checkbox>生成后打开文件所在目录</Checkbox>
  <div class="text-center">
    <Button color="red" class="me-2">确定</Button>
    <Button color="alternative">取消</Button>
  </div>
</Modal>

<Modal title="新增直播间" bind:open={addModal} size="xs" autoclose>
  <Label color={addErrorMsg ? "red" : "gray"}>
    房间号
    <Input
      bind:value={addRoom}
      color={addErrorMsg ? "red" : "base"}
      on:change={() => {
        if (!addRoom) {
          addErrorMsg = "";
          addValid = false;
          return;
        }
        // TODO preload room info
        const room_id = Number(addRoom);
        if (Number.isInteger(room_id) && room_id > 0) {
          addErrorMsg = "";
          addValid = true;
        } else {
          addErrorMsg = "房间号格式错误，请检查输入";
          addValid = false;
        }
      }}
    />
    {#if addErrorMsg}
      <Helper class="mt-2" color="red">
        <span class="font-medium">{addErrorMsg}</span>
      </Helper>
    {/if}
  </Label>
  <div class="text-center">
    <Button
      color="red"
      class="me-2"
      disabled={!addValid}
      on:click={() => {
        invoke("add_recorder", { roomId: Number(addRoom) }).catch(async (e) => {
          await message("请检查房间号是否有效：" + e, "添加失败");
        });
      }}>确定</Button
    >
    <Button color="alternative">取消</Button>
  </div>
</Modal>

<Modal title="直播间记录" bind:open={archiveModal} size="md">
  <Table>
    <TableHead>
      <TableHeadCell>直播时间</TableHeadCell>
      <TableHeadCell>操作</TableHeadCell>
    </TableHead>
    <TableBody tableBodyClass="divide-y">
      {#each archives as archive}
        <TableBodyRow>
          <TableBodyCell>{format_ts(archive)}</TableBodyCell>
          <TableBodyCell>
            <ButtonGroup>
              <Button
                on:click={() => {
                  invoke("open_live", {
                    roomId: archiveRoom.room_id,
                    ts: archive,
                  });
                }}>编辑切片</Button
              >
              <Button
                on:click={() => {
                  invoke("delete_archive", {
                    roomId: archiveRoom.room_id,
                    ts: archive,
                  }).then(async () => {
                    archives = await invoke("get_archives", {
                      roomId: archiveRoom.room_id,
                    });
                  });
                }}>移除</Button
              >
            </ButtonGroup>
          </TableBodyCell>
        </TableBodyRow>
      {/each}
    </TableBody>
  </Table>
</Modal>
