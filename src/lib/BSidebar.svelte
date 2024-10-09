<script>
    import {
        Sidebar,
        SidebarGroup,
        SidebarItem,
        SidebarWrapper,
    } from "flowbite-svelte";
    import {
        ChartPieSolid,
        GridSolid,
        MailBoxSolid,
        UserSolid,
        EditOutline,
        BookSolid,
        InfoCircleSolid,
        CogSolid,
    } from "flowbite-svelte-icons";
    let spanClass = "flex-1 ms-3 whitespace-nowrap";
    // acitveUrl is shared between project
    export let activeUrl = "#总览";
    export let room_count = 0;
    export let message_cnt = 0;
</script>

<Sidebar {activeUrl} asideClass="w-72 h-full z-[40]">
    <SidebarWrapper divClass="overflow-y-auto py-4 px-3 bg-gray-50 h-full">
        <SidebarGroup>
            <SidebarItem
                label="总览"
                href="#"
                on:click={() => {
                    activeUrl = "#总览";
                }}
            >
                <svelte:fragment slot="icon">
                    <ChartPieSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
            </SidebarItem>
            <SidebarItem
                label="直播间"
                href="#"
                on:click={() => {
                    activeUrl = "#直播间";
                }}
                {spanClass}
            >
                <svelte:fragment slot="icon">
                    <GridSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
                <svelte:fragment slot="subtext">
                    <span
                        class="inline-flex justify-center items-center px-2 ms-3 text-sm font-medium text-gray-800 bg-gray-200 rounded-full dark:bg-gray-700 dark:text-gray-300"
                    >
                        {room_count}
                    </span>
                </svelte:fragment>
            </SidebarItem>
            <SidebarItem
                label="消息"
                href="#"
                on:click={() => {
                    activeUrl = "#消息";
                }}
                {spanClass}
            >
                <svelte:fragment slot="icon">
                    <MailBoxSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
                <svelte:fragment slot="subtext">
                    <span
                        class="inline-flex justify-center items-center p-3 ms-3 w-3 h-3 text-sm font-medium text-primary-600 bg-primary-200 rounded-full dark:bg-primary-900 dark:text-primary-200"
                    >
                        {message_cnt}
                    </span>
                </svelte:fragment>
            </SidebarItem>
            <SidebarItem
                label="账号"
                href="#"
                on:click={() => {
                    activeUrl = "#账号";
                }}
            >
                <svelte:fragment slot="icon">
                    <UserSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
            </SidebarItem>
            <!-- <SidebarItem
        label="自动化"
        href="#"
        on:click={() => {
          activeUrl = "#自动化";
        }}
      >
        <svelte:fragment slot="icon">
          <EditOutline
            class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
          />
        </svelte:fragment>
      </SidebarItem> -->
            <SidebarItem
                label="设置"
                href="#"
                on:click={() => {
                    activeUrl = "#设置";
                }}
            >
                <svelte:fragment slot="icon">
                    <CogSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
            </SidebarItem>
        </SidebarGroup>
        <SidebarGroup border>
            <SidebarItem
                label="文档"
                href="#"
                on:click={() => {
                    activeUrl = "#文档";
                }}
            >
                <svelte:fragment slot="icon">
                    <BookSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
            </SidebarItem>
            <SidebarItem
                label="关于"
                href="#"
                on:click={() => {
                    activeUrl = "#关于";
                }}
            >
                <svelte:fragment slot="icon">
                    <InfoCircleSolid
                        class="w-6 h-6 text-gray-500 transition duration-75 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-white"
                    />
                </svelte:fragment>
            </SidebarItem>
        </SidebarGroup>
    </SidebarWrapper>
</Sidebar>
