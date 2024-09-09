<script lang="ts">
  import { invoke, convertFileSrc } from "@tauri-apps/api/tauri";
  import { fetch, ResponseType } from "@tauri-apps/api/http";
  import { message, open } from "@tauri-apps/api/dialog";
  import { open as shell_open } from "@tauri-apps/api/shell";
  import { exists } from "@tauri-apps/api/fs";
  import QRCode from "qrcode";
  interface Summary {
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
  let summary: Summary;
  async function setup() {
    console.log("setup");
    await update_summary();
    await get_config();
    setInterval(async () => {
      await update_summary();
    }, 2000);
  }

  async function update_summary() {
    let _summary: Summary = await invoke("get_summary");
    _summary.rooms = await Promise.all(
      _summary.rooms.map(async (room) => {
        room.user_avatar = await getImage(room.user_avatar);
        room.room_cover = await getImage(room.room_cover);
        room.room_keyframe = await getImage(room.room_keyframe);
        return room;
      }),
    );
    summary = _summary;
  }

  async function getImage(url: string) {
    if (!url) {
      return "";
    }
    const response = await fetch<Uint8Array>(url, {
      method: "GET",
      timeout: 30,
      responseType: ResponseType.Binary,
    });
    const binaryArray = new Uint8Array(response.data);
    var blob = new Blob([binaryArray], {
      type: response.headers["content-type"],
    });
    return URL.createObjectURL(blob);
  }
  setup();

  let add_model = {
    room_id: "",
  };

  async function add_room() {
    let room_id = parseInt(add_model.room_id);
    if (Number.isNaN(room_id) || room_id < 0) {
      await message("请输入正确的房间号", "无效的房间号");
      return;
    }
    invoke("add_recorder", { roomId: room_id }).catch(async (e) => {
      await message("请输入正确的房间号：" + e, "无效的房间号");
    });
  }

  async function remove_room(room_id: number) {
    await invoke("remove_recorder", { roomId: room_id });
  }

  let clip_model = {
    room: 0,
    title: "",
    max_len: 100,
    value: 30,
    loading: false,
    error: false,
    error_content: "",
    video: false,
    video_src: "",
  };

  async function clip(room: number, len: number) {
    return invoke("clip", { roomId: room, len: len });
  }

  async function show_in_folder(path: string) {
    await invoke("show_in_folder", { path });
  }

  let setting_model = {
    open: false,
    changed: false,
    cach_len: 300,
    cache_path: "",
    clip_path: "",
    admins: "",
    login: false,
    uid: "",
  };

  interface Config {
    admin_uid: number[];
    cache: string;
    max_len: number;
    output: string;
    login: boolean;
    uid: string;
  }

  async function get_config() {
    let config: Config = await invoke("get_config");
    setting_model.changed = false;
    setting_model.cach_len = config.max_len;
    setting_model.cache_path = config.cache;
    setting_model.clip_path = config.output;
    setting_model.admins = config.admin_uid.join(",");
    setting_model.login = config.login;
    setting_model.uid = config.uid;
  }

  async function apply_config() {
    await invoke("set_cache_path", { cachePath: setting_model.cache_path });
    await invoke("set_output_path", { outputPath: setting_model.clip_path });
    await invoke("set_max_len", { len: setting_model.cach_len });
    await invoke("set_admins", {
      admins: setting_model.admins.split(",").map((x) => parseInt(x)),
    });
  }

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
    console.log(qr_info);
  }

  async function check_qr() {
    let qr_status: { code: number; cookies: string } = await invoke(
      "get_qr_status",
      { qrcodeKey: oauth_key },
    );
    if (qr_status.code == 0) {
      clearInterval(check_interval);
      setting_model.login = true;
      setting_model.uid = qr_status.cookies.match(/DedeUserID=(\d+)/)[1];
      await invoke("set_cookies", { cookies: qr_status.cookies });
    }
  }
</script>

<div>
  <div>
    <table class="table w-full">
      <!-- head -->
      <thead>
        <tr>
          <th>直播间</th>
          <th class="text-center">缓存时长</th>
          <th class="text-center">状态</th>
          <th class="text-center">操作</th>
        </tr>
      </thead>
      <tbody>
        {#if summary}
          {#each summary.rooms as room}
            <tr>
              <td>
                <div class="flex items-center space-x-3">
                  <div>
                    <!-- svelte-ignore a11y-click-events-have-key-events -->
                    <div
                      class="flex w-48 h-27 cursor-pointer"
                      on:click={() => {
                        shell_open("https://live.bilibili.com/" + room.room_id);
                      }}
                    >
                      <img
                        src={room.room_cover}
                        alt={room.room_title}
                        on:mousemove={(e) => {
                          e.currentTarget.src = room.room_keyframe;
                        }}
                        on:mouseleave={(e) => {
                          e.currentTarget.src = room.room_cover;
                        }}
                      />
                    </div>
                  </div>
                  <div>
                    <span class="bold">{room.room_title}</span>
                    <br />
                    <span class="badge badge-neutral">房间号：{room.room_id}</span>
                  </div>
                </div>
              </td>
              <td class="text-center"
                ><div
                  class="radial-progress bg-primary text-primary-content border-4 border-primary"
                  style="--value:{100};"
                >
                  {Number(room.total_length).toFixed(1)}s
                </div></td
              >
              <td class="text-center">
                <span class="badge badge-neutral" class:badge-success={room.live_status}
                  >{room.live_status ? "直播中" : "未开播"}</span
                >
              </td>
              <td class="text-center">
                <div class="dropdown dropdown-end">
                  <!-- svelte-ignore a11y-no-noninteractive-tabindex -->
                  <div tabindex="0" class="btn m-1 btn-square btn-sm">
                    <svg
                      class="stroke-info w-full h-full"
                      viewBox="0 0 24 24"
                      fill="none"
                      xmlns="http://www.w3.org/2000/svg"
                    >
                      <path
                        fill-rule="evenodd"
                        clip-rule="evenodd"
                        d="M14.1395 12.0002C14.1395 13.1048 13.2664 14.0002 12.1895 14.0002C11.1125 14.0002 10.2395 13.1048 10.2395 12.0002C10.2395 10.8957 11.1125 10.0002 12.1895 10.0002C13.2664 10.0002 14.1395 10.8957 14.1395 12.0002Z"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                      <path
                        fill-rule="evenodd"
                        clip-rule="evenodd"
                        d="M7.57381 18.1003L5.12169 12.8133C4.79277 12.2907 4.79277 11.6189 5.12169 11.0963L7.55821 5.89229C7.93118 5.32445 8.55898 4.98876 9.22644 5.00029H12.1895H15.1525C15.8199 4.98876 16.4477 5.32445 16.8207 5.89229L19.2524 11.0923C19.5813 11.6149 19.5813 12.2867 19.2524 12.8093L16.8051 18.1003C16.4324 18.674 15.8002 19.0133 15.1281 19.0003H9.24984C8.5781 19.013 7.94636 18.6737 7.57381 18.1003Z"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  </div>
                  <!-- svelte-ignore a11y-no-noninteractive-tabindex -->
                  <ul
                    tabindex="0"
                    class="menu dropdown-content bg-base-100 rounded-box z-[1] w-52 p-2 shadow"
                  >
                    <li>
                      <a
                        href={"#"}
                        on:click={async (_) => {
                          await invoke("open_live", { roomId: room.room_id });
                        }}>查看直播流</a
                      >
                    </li>
                    <li>
                      <a
                        href={"#"}
                        on:click={(_) => {
                          clip_model.max_len = room.total_length;
                          clip_model.room = room.room_id;
                          clip_model.title = room.room_title;
                          clip_model.video = false;
                          //@ts-ignore save_modal is generated by dialog(id: save_modal)
                          save_modal.showModal();
                        }}>简易生成切片</a
                      >
                    </li>
                    <li>
                      <a
                        href={"#"}
                        on:click={() => {
                          remove_room(room.room_id).then(() => {
                            update_summary();
                          });
                        }}>移除</a
                      >
                    </li>
                  </ul>
                </div>
              </td>
            </tr>
          {/each}
        {:else}
          <tr>
            <progress class="progress w-56" />
          </tr>
        {/if}
      </tbody>
    </table>
    <div class="fixed bottom-6 right-6 flex flex-col">
      <div class="tooltip tooltip-left" data-tip="新增直播间">
        <label class="btn btn-circle" for="add-modal">
          <svg
            width="48px"
            height="48px"
            viewBox="-2.4 -2.4 28.80 28.80"
            fill="white"
            xmlns="http://www.w3.org/2000/svg"
            ><g id="SVGRepo_bgCarrier" stroke-width="0" /><g
              id="SVGRepo_tracerCarrier"
              stroke-linecap="round"
              stroke-linejoin="round"
            /><g id="SVGRepo_iconCarrier">
              <g id="Edit / Add_Plus">
                <path
                  id="Vector"
                  d="M6 12H12M12 12H18M12 12V18M12 12V6"
                  stroke="#ffffff"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </g>
            </g></svg
          >
        </label>
      </div>
      <div class="tooltip tooltip-left" data-tip="设置">
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <label
          class="btn btn-circle mt-2"
          for="setting-modal"
          on:click={() => get_config()}
        >
          <svg
            width="36px"
            height="36px"
            viewBox="0 0 24 24"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            ><g id="SVGRepo_bgCarrier" stroke-width="0" /><g
              id="SVGRepo_tracerCarrier"
              stroke-linecap="round"
              stroke-linejoin="round"
            /><g id="SVGRepo_iconCarrier">
              <path
                d="M9 22H15C20 22 22 20 22 15V9C22 4 20 2 15 2H9C4 2 2 4 2 9V15C2 20 4 22 9 22Z"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M15.5699 18.5001V14.6001"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M15.5699 7.45V5.5"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M15.57 12.65C17.0059 12.65 18.17 11.4859 18.17 10.05C18.17 8.61401 17.0059 7.44995 15.57 7.44995C14.134 7.44995 12.97 8.61401 12.97 10.05C12.97 11.4859 14.134 12.65 15.57 12.65Z"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M8.43005 18.5V16.55"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M8.43005 9.4V5.5"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
              <path
                d="M8.42996 16.5501C9.8659 16.5501 11.03 15.386 11.03 13.9501C11.03 12.5142 9.8659 11.3501 8.42996 11.3501C6.99402 11.3501 5.82996 12.5142 5.82996 13.9501C5.82996 15.386 6.99402 16.5501 8.42996 16.5501Z"
                stroke="#ffffff"
                stroke-width="1.5"
                stroke-miterlimit="10"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
            </g></svg
          >
        </label>
      </div>
    </div>
  </div>
  <input type="checkbox" id="add-modal" class="modal-toggle" />
  <label for="add-modal" class="modal cursor-pointer">
    <label class="modal-box relative" for="">
      <h3 class="text-lg font-bold mb-4">新增直播间</h3>
      <div class="flex justify-center">
        <input
          type="text"
          placeholder="输入直播间号"
          class="input input-bordered input-primary w-full max-w-xs mx-2"
          bind:value={add_model.room_id}
        />
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <label class="btn btn-primary" for="add-modal" on:click={add_room}
          >添加</label
        >
      </div>
    </label>
  </label>
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <dialog id="save_modal" class="modal cursor-pointer">
    <div class="modal-box relative">
      <h3 class="text-lg font-bold mb-4">生成切片 - {clip_model.title}</h3>
      {#if clip_model.video}
        <div class="mb-6">
          <!-- svelte-ignore a11y-media-has-caption -->
          <video src={convertFileSrc(clip_model.video_src)} controls />
        </div>
      {/if}
      <div class="flex flex-col items-center">
        最近 {clip_model.value}s
        <input
          type="range"
          min="10"
          max={clip_model.max_len}
          bind:value={clip_model.value}
          class="range range-primary mt-4"
        />
        <div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-label-has-associated-control -->
          <label
            class="btn btn-primary my-4"
            class:loading={clip_model.loading}
            on:click={() => {
              clip_model.loading = true;
              clip(clip_model.room, clip_model.value)
                .then((f) => {
                  exists(String(f)).then((result) => {
                    clip_model.loading = false;
                    if (result) {
                      clip_model.error = false;
                      clip_model.video = true;
                      clip_model.video_src = String(f);
                    } else {
                      clip_model.error = true;
                      clip_model.error_content = "生成失败，请重试";
                    }
                  });
                })
                .catch((e) => {
                  clip_model.loading = false;
                  clip_model.error = true;
                  clip_model.error_content = e;
                });
            }}>生成切片</label
          >
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <label
            class="btn btn-secondary"
            for=""
            on:click={(_) => {
              show_in_folder(setting_model.clip_path);
            }}>打开切片文件夹</label
          >
        </div>
        {#if clip_model.error}
          <div class="alert alert-error shadow-lg">
            <div>
              <svg
                xmlns="http://www.w3.org/2000/svg"
                class="stroke-current flex-shrink-0 h-6 w-6"
                fill="none"
                viewBox="0 0 24 24"
                ><path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z"
                /></svg
              >
              <span>生成切片失败：{clip_model.error_content}</span>
            </div>
          </div>
        {/if}
      </div>
    </div>
    <form method="dialog" class="modal-backdrop">
      <button>close</button>
    </form>
  </dialog>

  <!-- Setting modal Part -->
  <input
    type="checkbox"
    id="setting-modal"
    class="modal-toggle"
    bind:checked={setting_model.open}
  />
  <label for="setting-modal" class="modal cursor-pointer backdrop-blur-sm">
    <label class="modal-box relative" for="">
      <h3 class="text-lg font-bold">设置</h3>
      <div class="flex flex-col">
        {#if setting_model.login}
          <div class="flex items-center flex-col">
            <div class="badge badge-secondary">
              已登录（UID：{setting_model.uid}）
            </div>
            <button
              class="btn btn-sm btn-warning my-4"
              on:click={() => {
                setting_model.login = false;
                invoke("logout");
              }}>退出登录</button
            >
          </div>
        {:else}
          <div class="flex items-center flex-col">
            <canvas id="qr" style="display: none;"></canvas>
            <button
              class="btn btn-sm btn-primary my-4"
              on:click={() => {
                handle_qr();
              }}>获取/刷新登录二维码</button
            >
          </div>
        {/if}
        <div class="divider"></div>
        <label class="flex items-center my-2" for=""
          >缓存目录：
          <button
            class="btn btn-outline rounded-e-none h-[32px] min-h-[32px]"
            on:click={async () => {
              const output_path = await open({
                title: "选择缓存目录",
                directory: true,
              });
              setting_model.cache_path = Array.isArray(output_path) ? output_path[0] : output_path;
              setting_model.changed = true;
            }}>选择目录</button>
            <input
            type="text"
            class="input input-sm input-bordered rounded-s-none"
            bind:value={setting_model.cache_path}
            on:change={() => {
              setting_model.changed = true;
            }}
          /></label
        >
        <label class="flex items-center my-2" for=""
          >切片目录：
          <button
            class="btn btn-outline rounded-e-none h-[32px] min-h-[32px]"
            on:click={async () => {
              const output_path = await open({
                title: "选择切片保存目录",
                directory: true,
              });
              setting_model.clip_path = Array.isArray(output_path) ? output_path[0] : output_path;
              setting_model.changed = true;
            }}>选择目录</button>
            <input
            type="text"
            class="input input-sm input-bordered rounded-s-none"
            bind:value={setting_model.clip_path}
            on:change={() => {
              setting_model.changed = true;
            }}
          />
        </label
        >
        <label class="flex items-center my-2" for=""
          >管理员UID：<input
            type="text"
            class="input input-sm input-bordered"
            bind:value={setting_model.admins}
            on:change={() => {
              setting_model.changed = true;
            }}
          /></label
        >
        <div class="text-sm text-slate-500">
          相关说明：管理员 UID 可添加多个，使用 “,” 分隔。设定为管理员的用户可以在直播间发送
          <div class="badge badge-outline">/clip + 时长</div>
          弹幕来触发切片， 例如：
          <div class="badge badge-outline">/clip 30</div>
          将会保存最近的 30s 录播
        </div>
        <button
          class="btn btn-sm btn-primary my-4"
          disabled={!setting_model.changed}
          on:click={() => {
            apply_config();
            setting_model.open = false;
          }}>应用</button
        >
      </div>
    </label>
  </label>
</div>

<style>
</style>
