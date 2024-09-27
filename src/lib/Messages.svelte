<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import {
    Table,
    TableBody,
    TableBodyRow,
    TableBodyCell,
    Button,
  } from "flowbite-svelte";
  import type { MessageItem } from "./db";
  import { CloseCircleSolid, InfoCircleSolid } from "flowbite-svelte-icons";

  export let message_cnt = 0;
  let messages: MessageItem[] = [];
  async function update() {
    messages = ((await invoke("get_messages")) as MessageItem[]).sort(
      (a, b) => b.id - a.id,
    );
    message_cnt = messages.length;
  }
  update();
  setInterval(update, 1000);

  async function delete_message(id: number) {
    await invoke("delete_message", { id: id });
    await update();
  }
</script>

<div class="p-8 pt-12 h-full overflow-auto">
  <Table hoverable={true} divClass="relative max-h-full overflow-hidden" shadow>
    <TableBody tableBodyClass="divide-y">
      {#each messages as message}
        <TableBodyRow>
          <TableBodyCell tdClass="pl-6 py-4 text-center">
            <InfoCircleSolid class="w-8 h-8" />
          </TableBodyCell>
          <TableBodyCell tdClass="text-wrap px-6 py-4">
            <p class="text-lg font-bold">{message.title}</p>
            <p class="text-slate-500">{message.content}</p>
          </TableBodyCell>
          <TableBodyCell tdClass="px-6 py-4 text-end"
            ><p class="text-slate-400">
              {new Date(message.created_at).toLocaleString()}
            </p></TableBodyCell
          >
          <TableBodyCell tdClass="px-6 py-4 text-end">
            <Button
              class="!p-2"
              size="sm"
              color="red"
              on:click={async () => {
                await delete_message(message.id);
              }}
            >
              <CloseCircleSolid />
            </Button>
          </TableBodyCell>
        </TableBodyRow>
      {/each}
    </TableBody>
  </Table>
</div>
