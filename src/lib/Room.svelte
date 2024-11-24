<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { message } from "@tauri-apps/plugin-dialog";
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
    Tooltip,
  } from "flowbite-svelte";
  import {
    ChevronDownOutline,
    PlusOutline,
    ExclamationCircleOutline,
  } from "flowbite-svelte-icons";
  import type { RecorderList } from "./interface";
  import Image from "./Image.svelte";
  import type { RecordItem } from "./db";

  export let room_count = 0;
  let summary: RecorderList = {
    count: 0,
    recorders: [],
  };

  async function update_summary() {
    summary = (await invoke("get_recorder_list")) as RecorderList;
    room_count = summary.count;
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
  let archives: RecordItem[] = [];
  async function showArchives(room_id: number) {
    archives = await invoke("get_archives", { roomId: room_id });
    archiveModal = true;
    console.log(archives);
  }
  function format_ts(ts_string: string) {
    const date = new Date(ts_string);
    return date.toLocaleString();
  }
  function format_duration(duration: number) {
    const hours = Math.floor(duration / 3600)
      .toString()
      .padStart(2, "0");
    const minutes = Math.floor((duration % 3600) / 60)
      .toString()
      .padStart(2, "0");
    const seconds = (duration % 60).toString().padStart(2, "0");

    return `${hours}:${minutes}:${seconds}`;
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
  function calc_bitrate(size: number, duration: number) {
    return ((size * 8) / duration / 1024).toFixed(0);
  }
</script>

<div class="p-8 pt-12 h-full overflow-auto">
  <Table hoverable={true} divClass="relative max-h-full" shadow>
    <TableHead>
      <TableHeadCell>房间号</TableHeadCell>
      <TableHeadCell>标题</TableHeadCell>
      <TableHeadCell>用户</TableHeadCell>
      <TableHeadCell>状态</TableHeadCell>
      <TableHeadCell>缓存时长</TableHeadCell>
      <TableHeadCell>
        <span class="sr-only">Edit</span>
      </TableHeadCell>
    </TableHead>
    <TableBody tableBodyClass="divide-y">
      {#each summary.recorders as room}
        <TableBodyRow>
          <TableBodyCell>{room.room_id}</TableBodyCell>
          <TableBodyCell>{room.room_info.room_title}</TableBodyCell>
          <TableBodyCell>
            <div class="pr-4">
              <Image
                iclass="rounded-full w-12 inline"
                src={room.user_info.user_avatar_url}
              />
              <span>
                {room.user_info.user_name}
              </span>
            </div>
          </TableBodyCell>
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
                <!-- <DropdownItem
                                    on:click={() => {
                                        quickClipRoom = room.room_id;
                                        quickClipSelected = 30;
                                        quickClipModal = true;
                                    }}>快速切片</DropdownItem
                                > -->
              {/if}
              <DropdownItem
                on:click={() => {
                  archiveRoom = room;
                  showArchives(room.room_id);
                }}>查看历史记录</DropdownItem
              >
              <DropdownItem
                class="text-red-500"
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

  <div class="fixed end-4 bottom-4">
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
        确定要移除这个直播间吗？
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
          invoke("add_recorder", { roomId: Number(addRoom) }).catch(
            async (e) => {
              await message("请检查房间号是否有效：" + e, "添加失败");
            },
          );
        }}>确定</Button
      >
      <Button color="alternative">取消</Button>
    </div>
  </Modal>

  <Modal title="直播间记录" bind:open={archiveModal} size="lg">
    <Table>
      <TableHead>
        <TableHeadCell>直播时间</TableHeadCell>
        <TableHeadCell>标题</TableHeadCell>
        <TableHeadCell>时长</TableHeadCell>
        <TableHeadCell>缓存</TableHeadCell>
        <TableHeadCell>操作</TableHeadCell>
      </TableHead>
      <TableBody tableBodyClass="divide-y">
        {#each archives as archive}
          <TableBodyRow>
            <TableBodyCell>{format_ts(archive.created_at)}</TableBodyCell>
            <TableBodyCell>{archive.title}</TableBodyCell>
            <TableBodyCell>{format_duration(archive.length)}</TableBodyCell>
            <TableBodyCell>
              <span>{format_size(archive.size)}</span>
            </TableBodyCell>
            <TableBodyCell>
              <ButtonGroup>
                <Button
                  on:click={() => {
                    invoke("open_live", {
                      roomId: archiveRoom.room_id,
                      ts: archive.live_id,
                    });
                  }}>编辑切片</Button
                >
                <Button
                  color="red"
                  on:click={() => {
                    invoke("delete_archive", {
                      roomId: archiveRoom.room_id,
                      ts: archive.live_id,
                    })
                      .then(async () => {
                        archives = await invoke("get_archives", {
                          roomId: archiveRoom.room_id,
                        });
                      })
                      .catch((e) => {
                        alert(e);
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
</div>
