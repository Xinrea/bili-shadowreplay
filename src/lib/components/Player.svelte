<script lang="ts" context="module">
  declare const shaka: any;
</script>

<script lang="ts">
  import { invoke, TAURI_ENV, ENDPOINT, listen, log } from "../invoker";
  import type { AccountInfo } from "../db";
  import type { Marker, RecorderList, RecorderInfo, Range } from "../interface";

  import { createEventDispatcher } from "svelte";
  import {
    GridOutline,
    SortHorizontalOutline,
    FileExportOutline,
  } from "flowbite-svelte-icons";
  import { save } from "@tauri-apps/plugin-dialog";
  const dispatch = createEventDispatcher();
  const DANMU_STATISTIC_GAP = 5;

  interface DanmuEntry {
    ts: number;
    content: string;
  }

  export let platform: string;
  export let room_id: string;
  export let live_id: string;
  export let ranges: Range[] = [];
  export let global_offset = 0;
  export let focus_start = 0;
  export let focus_end = 0;
  export let markers: Marker[] = [];
  export let danmu_records: DanmuEntry[] = [];
  export function seek(offset: number) {
    video.currentTime = offset;
  }
  let video: HTMLVideoElement;
  let show_detail = false;
  let show_list = false;
  let show_export = false;
  let recorders: RecorderInfo[] = [];

  let start = 0;
  let end = 0;
  let currentRangeIndex: number = -1; // 当前正在编辑的区间索引，-1 表示没有区间

  // local setting of danmu offset
  let local_offset: number =
    parseInt(localStorage.getItem(`local_offset:${live_id}`) || "0", 10) || 0;

  // 获取当前区间
  function getCurrentRange(): Range | null {
    if (currentRangeIndex >= 0 && currentRangeIndex < ranges.length) {
      return ranges[currentRangeIndex];
    }
    return null;
  }

  // 检测两个区间是否重叠或相邻（允许小误差）
  function rangesOverlap(range1: Range, range2: Range): boolean {
    // 允许 0.1 秒的误差，认为是相邻的区间
    return !(
      range1.end < range2.start - 0.1 || range1.start > range2.end + 0.1
    );
  }

  // 合并两个重叠的区间
  function mergeRanges(range1: Range, range2: Range): Range {
    return {
      start: Math.min(range1.start, range2.start),
      end: Math.max(range1.end, range2.end),
    };
  }

  // 合并所有重叠的区间
  function mergeOverlappingRanges(): void {
    if (ranges.length <= 1) return;

    const beforeCount = ranges.length;

    // 按开始时间排序
    ranges.sort((a, b) => a.start - b.start);

    const merged: Range[] = [];
    let current = ranges[0];

    for (let i = 1; i < ranges.length; i++) {
      const next = ranges[i];

      // 如果当前区间与下一个区间重叠或相邻，合并它们
      if (rangesOverlap(current, next)) {
        current = mergeRanges(current, next);
      } else {
        merged.push(current);
        current = next;
      }
    }
    merged.push(current);

    ranges = merged;

    const afterCount = ranges.length;
    if (beforeCount > afterCount) {
      console.log(`Merged overlapping ranges: ${beforeCount} -> ${afterCount}`);
    }
  }

  // 保存区间数组到 localStorage
  // 注意：区间始终保存为绝对时间（不依赖 focus_start），这样在切换 focus 模式时不会丢失
  function saveRanges() {
    // 将相对时间转换为绝对时间保存
    const rangesToSave = ranges.map((r) => ({
      start: r.start + focus_start,
      end: r.end + focus_start,
      activated: r.activated,
    }));
    localStorage.setItem(`${live_id}_ranges`, JSON.stringify(rangesToSave));
    localStorage.setItem(
      `${live_id}_currentRangeIndex`,
      currentRangeIndex.toString()
    );
    console.log(
      "Saved ranges (absolute time):",
      rangesToSave,
      "current index:",
      currentRangeIndex
    );
  }

  // 从 localStorage 加载区间数组
  // 注意：加载时保留所有区间，但只显示在当前 focus 范围内的区间
  function loadRanges() {
    const saved = localStorage.getItem(`${live_id}_ranges`);
    if (saved) {
      try {
        const savedRanges = JSON.parse(saved) as Array<{
          start: number;
          end: number;
          activated?: boolean;
        }>;
        // 保存的区间是绝对时间，需要转换为相对时间
        // 但保留所有区间，不进行过滤，因为区间可能跨越多个 focus 范围
        ranges = savedRanges.map((r) => ({
          start: r.start - focus_start,
          end: r.end - focus_start,
          activated: r.activated !== false, // 默认为 true
        }));

        // 不在这里过滤区间，因为区间可能不在当前 focus 范围内
        // 但需要确保区间的时间顺序正确（start < end）
        ranges = ranges.filter((r) => r.end > r.start);

        const savedIndex = localStorage.getItem(`${live_id}_currentRangeIndex`);
        if (savedIndex) {
          const index = parseInt(savedIndex, 10);
          // 检查索引是否有效，如果对应的区间不在当前 focus 范围内，重置索引
          if (index >= 0 && index < ranges.length) {
            const range = ranges[index];
            // 如果区间在当前 focus 范围内（至少部分可见），保留索引
            if (
              range.end > 0 &&
              range.start < (focus_end - focus_start || Infinity)
            ) {
              currentRangeIndex = index;
            } else {
              // 否则找到第一个在当前范围内的区间，或设为 -1
              const visibleIndex = ranges.findIndex(
                (r) =>
                  r.end > 0 && r.start < (focus_end - focus_start || Infinity)
              );
              currentRangeIndex = visibleIndex >= 0 ? visibleIndex : -1;
            }
          } else {
            currentRangeIndex = ranges.length > 0 ? 0 : -1;
          }
        } else {
          currentRangeIndex = ranges.length > 0 ? 0 : -1;
        }
      } catch (e) {
        console.error("Failed to load ranges:", e);
        ranges = [];
        currentRangeIndex = -1;
      }
    }
    // 兼容旧版本的单个区间数据
    if (ranges.length === 0) {
      const oldStart = localStorage.getItem(`${live_id}_start`);
      const oldEnd = localStorage.getItem(`${live_id}_end`);
      if (oldStart && oldEnd) {
        const s = parseFloat(oldStart) - focus_start;
        const e = parseFloat(oldEnd) - focus_start;
        if (e > s) {
          ranges = [{ start: s, end: e, activated: true }];
          currentRangeIndex = 0;
          saveRanges();
        }
      }
    }
  }

  // 向后兼容：保持 start 和 end 的 getter/setter
  $: {
    const current = getCurrentRange();
    if (current) {
      start = current.start;
      end = current.end;
    } else {
      start = 0;
      end = 0;
    }
  }

  async function load_metadata(url: string) {
    let offset = 0;
    let is_fmp4 = false;
    const response = await fetch(url);
    const m3u8Content = await response.text();

    // extract offset from m3u8
    const firstSegmentDatetime = m3u8Content
      .split("\n")
      .find((line) => line.startsWith("#EXT-X-PROGRAM-DATE-TIME:"));
    if (firstSegmentDatetime) {
      const date_str = firstSegmentDatetime.replace(
        "#EXT-X-PROGRAM-DATE-TIME:",
        ""
      );
      offset = new Date(date_str).getTime() / 1000;
    } else {
      offset = parseInt(live_id) / 1000;
    }

    // check if fmp4 live
    if (m3u8Content.includes("#EXT-X-MAP:URI=")) {
      is_fmp4 = true;
    }

    return {
      offset,
      is_fmp4,
    };
  }

  function createMasterPlaylist(mediaPlaylistUrl: string) {
    return `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-STREAM-INF:BANDWIDTH=10000000,CODECS="avc1.64002a,mp4a.40.2"
${mediaPlaylistUrl}`;
  }

  function tauriNetworkPlugin(uri, requestType, progressUpdated) {
    const controller = new AbortController();
    const abortStatus = {
      canceled: false,
      timedOut: false,
    };

    const pendingRequest = new Promise((resolve, reject) => {
      invoke("fetch_hls", { uri: uri })
        .then((data: number[]) => {
          if (abortStatus.canceled) {
            reject(new Error("Request was aborted"));
            return;
          }

          // Convert to Uint8Array first to ensure proper byte array
          const uint8Array = new Uint8Array(data);
          const arrayBuffer = uint8Array.buffer;

          const is_m3u8 = uri.split("?")[0].endsWith(".m3u8");

          if (is_m3u8) {
            let m3u8Content = new TextDecoder().decode(uint8Array);
            // get first segment DATETIME
            // #EXT-X-PROGRAM-DATE-TIME:2025-09-20T20:04:39.000+08:00
            const firstSegmentDatetime = m3u8Content
              .split("\n")
              .find((line) => line.startsWith("#EXT-X-PROGRAM-DATE-TIME:"));
            if (firstSegmentDatetime) {
              if (global_offset == 0) {
                const date_str = firstSegmentDatetime.replace(
                  "#EXT-X-PROGRAM-DATE-TIME:",
                  ""
                );
                global_offset = new Date(date_str).getTime() / 1000;
              }
            } else {
              if (global_offset == 0) {
                global_offset = parseInt(live_id) / 1000;
              }
            }
          }

          // Set content-type based on URI extension
          let content_type = is_m3u8
            ? "application/vnd.apple.mpegurl"
            : "application/octet-stream";

          // Create response object with byteLength for segment data
          const response = {
            uri: uri,
            originalUri: uri,
            status: 200,
            data: arrayBuffer,
            headers: {
              "content-type": content_type,
            },
            timeMs: Date.now(),
            byteLength: arrayBuffer.byteLength,
          };

          resolve(response);
        })
        .catch((error) => {
          log.error("tauriNetworkPlugin error for URI:", uri, error);
          reject(
            new shaka.util.Error(
              shaka.util.Error.Severity.CRITICAL,
              shaka.util.Error.Category.NETWORK,
              shaka.util.Error.Code.OPERATION_ABORTED,
              error.message || error || "Network request failed"
            )
          );
        });
    });

    return new shaka.util.AbortableOperation(pendingRequest, () => {
      abortStatus.canceled = true;
      controller.abort();
      return Promise.resolve();
    });
  }

  if (TAURI_ENV) {
    console.log("register tauri network plugin");
    shaka.net.NetworkingEngine.registerScheme("http", tauriNetworkPlugin);
    shaka.net.NetworkingEngine.registerScheme("https", tauriNetworkPlugin);
    shaka.net.NetworkingEngine.registerScheme("tauri", tauriNetworkPlugin);
  }

  async function update_stream_list() {
    recorders = (
      (await invoke("get_recorder_list")) as RecorderList
    ).recorders.filter(
      (r) => r.room_info.status && r.room_info.room_id != room_id
    );
  }

  function go_to(platform: string, room_id: string, live_id: string) {
    const url = `${window.location.origin}${window.location.pathname}?platform=${platform}&room_id=${room_id}&live_id=${live_id}`;
    window.location.href = url;
  }

  function zoomOnRange(start: number, end: number) {
    const url = `${window.location.origin}${window.location.pathname}?platform=${platform}&room_id=${room_id}&live_id=${live_id}&start=${start}&end=${end}`;
    window.location.href = url;
  }

  function resetZoom() {
    const url = `${window.location.origin}${window.location.pathname}?platform=${platform}&room_id=${room_id}&live_id=${live_id}`;
    window.location.href = url;
  }

  async function init() {
    update_stream_list();

    setInterval(async () => {
      await update_stream_list();
    }, 5 * 1000);

    video = document.getElementById("video") as HTMLVideoElement;
    video.crossOrigin = "anonymous";
    const ui = video["ui"];
    const controls = ui.getControls();
    const player = controls.getPlayer();

    const config = {
      enableKeyboardPlaybackControls: false,
      seekBarColors: {
        base: "rgba(255,255,255,.2)",
        buffered: "rgba(255,255,255,.4)",
        played: "rgb(255,0,0)",
      },
    };
    ui.configure(config);
    // Attach player and UI to the window to make it easy to access in the JS console.
    (window as any).player = player;
    (window as any).ui = ui;

    player.configure({
      streaming: {
        lowLatencyMode: true,
      },
      cmsd: {
        enabled: false,
      },
    });

    player.addEventListener("ended", async () => {
      // prevent endless reload
      setTimeout(location.reload, 3 * 1000);
    });
    player.addEventListener("manifestloaded", (event) => {
      console.log("Manifest loaded:", event);
    });

    try {
      let direct_url = `${ENDPOINT ? ENDPOINT : window.location.origin}/hls/${platform}/${room_id}/${live_id}/playlist.m3u8?start=${focus_start}&end=${focus_end}`;
      if (!TAURI_ENV) {
        const { offset, is_fmp4 } = await load_metadata(direct_url);
        global_offset = offset;
        if (is_fmp4) {
          let master_url = createMasterPlaylist(direct_url);
          let blob = new Blob([master_url], {
            type: "application/vnd.apple.mpegurl",
          });
          master_url = URL.createObjectURL(blob);
          await player.load(master_url);
        } else {
          await player.load(direct_url);
        }
      } else {
        await player.load(direct_url);
      }

      // This runs if the asynchronous load is successful.
      console.log("The video has now been loaded!");
    } catch (error) {
      log.error("Error code", error.code, "object", error);
      if (error.code == 3000) {
        // reload
        setTimeout(() => {
          location.reload();
        }, 1 * 1000);
      } else {
        alert(
          "加载失败，请尝试刷新页面\n" +
            "Error code: " +
            error.code +
            "\n" +
            "Error message: " +
            error.message
        );
        log.error("Error code", error.code, "object", error);
      }
    }

    // init video volume from localStorage
    let localVolume = localStorage.getItem(`volume:${room_id}`);
    if (localVolume != undefined) {
      console.log("Load local volume", localVolume);
      video.volume = parseFloat(localVolume);
    }

    video.addEventListener("volumechange", (event) => {
      localStorage.setItem(`volume:${room_id}`, video.volume.toString());
    });

    document.getElementsByClassName("shaka-overflow-menu-button")[0].remove();
    document.getElementsByClassName("shaka-fullscreen-button")[0].remove();
    // add self-defined element in shaka-bottom-controls.shaka-no-propagation (second seekbar)
    const shakaBottomControls = document.querySelector(
      ".shaka-bottom-controls.shaka-no-propagation"
    );
    const selfSeekbar = document.createElement("div");
    selfSeekbar.className = "shaka-seek-bar shaka-no-propagation";
    selfSeekbar.innerHTML = `
          <div class="shaka-seek-bar-container self-defined" style="background-color: gray; margin: 0px 10px; box-sizing: initial; top: -8px;">
            <div class="shaka-seek-bar shaka-no-propagation">
              <div class="shaka-seek-bar-buffered" style="width: 0%;"></div>
              <div class="shaka-seek-bar-played" style="width: 0%;"></div>
              <div class="shaka-seek-bar-hover" style="transform: translateX(0px);"></div>
              <div class="shaka-seek-bar-hit-target"></div>
            </div>
          </div>
        `;
    shakaBottomControls.appendChild(selfSeekbar);

    // add to shaka-spacer
    const shakaSpacer = document.querySelector(".shaka-spacer") as HTMLElement;

    let danmu_enabled = true;
    // get danmaku record
    danmu_records = (await invoke("get_danmu_record", {
      roomId: room_id,
      liveId: live_id,
      platform: platform,
    })) as DanmuEntry[];

    console.log("danmu loaded:", danmu_records.length);

    let danmu_displayed = {};
    // history danmaku sender
    setInterval(() => {
      if (video.paused || !danmu_enabled || danmu_records.length == 0) {
        return;
      }

      // using live source
      if (isLive() && get_total() - video.currentTime <= 5) {
        return;
      }

      const cur = Math.floor(
        (video.currentTime + focus_start + local_offset + global_offset) * 1000
      );

      let danmus = danmu_records.filter((v) => {
        return v.ts >= cur - 1000 && v.ts < cur;
      });
      danmus.forEach((v) => {
        if (danmu_displayed[v.ts]) {
          delete danmu_displayed[v.ts];
          return;
        }
        danmu_handler(v.content);
      });
    }, 1000);

    if (isLive()) {
      if (platform == "bilibili") {
        // add a account select
        const accountSelect = document.createElement("select");
        accountSelect.style.height = "30px";
        accountSelect.style.minWidth = "100px";
        accountSelect.style.backgroundColor = "rgba(0, 0, 0, 0)";
        accountSelect.style.color = "white";
        accountSelect.style.border = "1px solid gray";
        accountSelect.style.padding = "0 10px";
        accountSelect.style.boxSizing = "border-box";
        accountSelect.style.fontSize = "1em";
        // get accounts from tauri
        const account_info = (await invoke("get_accounts")) as AccountInfo;
        account_info.accounts.forEach((account) => {
          if (account.platform !== "bilibili") {
            return;
          }
          const option = document.createElement("option");
          option.value = account.uid.toString();
          option.text = account.name;
          accountSelect.appendChild(option);
        });
        // add a danmaku send input
        const danmakuInput = document.createElement("input");
        danmakuInput.type = "text";
        danmakuInput.placeholder = "回车发送弹幕";
        danmakuInput.style.width = "30%";
        danmakuInput.style.height = "30px";
        danmakuInput.style.backgroundColor = "rgba(0, 0, 0, 0)";
        danmakuInput.style.color = "white";
        danmakuInput.style.border = "1px solid gray";
        danmakuInput.style.padding = "0 10px";
        danmakuInput.style.boxSizing = "border-box";
        danmakuInput.style.fontSize = "1em";
        danmakuInput.addEventListener("keydown", async (e) => {
          if (e.key === "Enter") {
            const value = danmakuInput.value;
            if (value) {
              // get account uid from select
              const uid = accountSelect.value;
              await invoke("send_danmaku", {
                uid,
                roomId: room_id,
                message: value,
              });
              danmakuInput.value = "";
            }
          }
        });

        shakaSpacer.appendChild(accountSelect);
        shakaSpacer.appendChild(danmakuInput);
      }

      // listen to danmaku event
      await listen(`danmu:${room_id}`, (event: { payload: DanmuEntry }) => {
        if (global_offset == 0) {
          return;
        }

        if (event.payload.ts < global_offset * 1000) {
          log.error("invalid danmu ts:", event.payload.ts, global_offset);
          return;
        }

        let danmu_record = event.payload;
        // if not enabled or playback is not keep up with live, ignore the danmaku
        if (!danmu_enabled || get_total() - video.currentTime > 5) {
          danmu_records = [...danmu_records, danmu_record];
          return;
        }
        if (Object.keys(danmu_displayed).length > 1000) {
          danmu_displayed = {};
        }
        danmu_displayed[event.payload.ts] = true;
        danmu_records = [...danmu_records, danmu_record];
        danmu_handler(danmu_record.content);
      });
    }

    // create a danmaku toggle button
    const danmakuToggle = document.createElement("button");
    danmakuToggle.innerText = "弹幕已开启";
    danmakuToggle.style.height = "30px";
    danmakuToggle.style.backgroundColor = "rgba(0, 128, 255, 0.5)";
    danmakuToggle.style.color = "white";
    danmakuToggle.style.border = "1px solid gray";
    danmakuToggle.style.padding = "0 10px";
    danmakuToggle.style.boxSizing = "border-box";
    danmakuToggle.style.fontSize = "1em";
    danmakuToggle.addEventListener("click", async () => {
      danmu_enabled = !danmu_enabled;
      danmakuToggle.innerText = danmu_enabled ? "弹幕已开启" : "弹幕已关闭";
      // clear background color
      danmakuToggle.style.backgroundColor = danmu_enabled
        ? "rgba(0, 128, 255, 0.5)"
        : "rgba(255, 0, 0, 0.5)";
    });

    // create a area that overlay half top of the video, which shows danmakus floating from right to left
    const overlay = document.createElement("div");
    overlay.style.width = "100%";
    overlay.style.height = "100%";
    overlay.style.position = "absolute";
    overlay.style.top = "0";
    overlay.style.left = "0";
    overlay.style.pointerEvents = "none";
    overlay.style.zIndex = "30";
    overlay.style.display = "flex";
    overlay.style.alignItems = "center";
    overlay.style.flexDirection = "column";
    overlay.style.paddingTop = "10%";
    // place overlay to the top of the video
    video.parentElement.appendChild(overlay);

    // Store the positions of the last few danmakus to avoid overlap
    const danmakuPositions = [];

    function danmu_handler(content: string) {
      const danmaku = document.createElement("p");
      danmaku.style.position = "absolute";

      // Calculate a random position for the danmaku
      let topPosition = 0;
      let attempts = 0;
      do {
        topPosition = Math.random() * 30;
        attempts++;
      } while (
        danmakuPositions.some((pos) => Math.abs(pos - topPosition) < 5) &&
        attempts < 10
      );

      // Record the position
      danmakuPositions.push(topPosition);
      if (danmakuPositions.length > 10) {
        danmakuPositions.shift(); // Keep the last 10 positions
      }

      danmaku.style.top = `${topPosition}%`;
      danmaku.style.right = "0";
      danmaku.style.color = "white";
      danmaku.style.fontSize = "1.2em";
      danmaku.style.whiteSpace = "nowrap";
      danmaku.style.transform = "translateX(100%)";
      danmaku.style.transition = "transform 10s linear";
      danmaku.style.pointerEvents = "none";
      danmaku.style.margin = "0";
      danmaku.style.padding = "0";
      danmaku.style.zIndex = "500";
      danmaku.style.textShadow = "1px 1px 2px rgba(0, 0, 0, 0.6)";
      danmaku.innerText = content;
      overlay.appendChild(danmaku);
      requestAnimationFrame(() => {
        danmaku.style.transform = `translateX(-${overlay.clientWidth + danmaku.clientWidth}px)`;
      });
      danmaku.addEventListener("transitionend", () => {
        overlay.removeChild(danmaku);
      });
    }

    shakaSpacer.appendChild(danmakuToggle);

    // create a playback rate button and menu
    const playbackRateButton = document.createElement("button");
    playbackRateButton.style.height = "30px";
    playbackRateButton.style.minWidth = "30px";
    playbackRateButton.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
    playbackRateButton.style.color = "white";
    playbackRateButton.style.border = "1px solid gray";
    playbackRateButton.style.padding = "0 10px";
    playbackRateButton.style.boxSizing = "border-box";
    playbackRateButton.style.fontSize = "1em";
    playbackRateButton.style.right = "10px";
    playbackRateButton.style.position = "absolute";
    playbackRateButton.innerText = "⚙️";

    const SettingMenu = document.createElement("div");
    SettingMenu.style.position = "absolute";
    SettingMenu.style.bottom = "40px";
    SettingMenu.style.right = "10px";
    SettingMenu.style.backgroundColor = "rgba(0, 0, 0, 0.8)";
    SettingMenu.style.border = "1px solid gray";
    SettingMenu.style.padding = "8px";
    SettingMenu.style.display = "none";
    SettingMenu.style.zIndex = "1000";

    // Add danmaku offset input
    const offsetContainer = document.createElement("div");
    offsetContainer.style.marginBottom = "8px";

    const offsetLabel = document.createElement("label");
    offsetLabel.innerText = "弹幕偏移(秒):";
    offsetLabel.style.color = "white";
    offsetLabel.style.marginRight = "8px";

    const offsetInput = document.createElement("input");
    offsetInput.type = "number";
    offsetInput.value = "0";
    offsetInput.style.width = "60px";
    offsetInput.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
    offsetInput.style.color = "white";
    offsetInput.style.border = "1px solid gray";
    offsetInput.style.padding = "2px 4px";
    offsetInput.style.boxSizing = "border-box";

    offsetContainer.appendChild(offsetLabel);
    offsetContainer.appendChild(offsetInput);
    SettingMenu.appendChild(offsetContainer);

    // Add divider
    const divider = document.createElement("hr");
    divider.style.border = "none";
    divider.style.borderTop = "1px solid gray";
    divider.style.margin = "8px 0";
    SettingMenu.appendChild(divider);

    // Add playback rate options
    const rates = [0.5, 1, 1.5, 2, 5];
    rates.forEach((rate) => {
      const rateButton = document.createElement("button");
      rateButton.innerText = `${rate}x`;
      rateButton.style.display = "block";
      rateButton.style.width = "100%";
      rateButton.style.padding = "4px 8px";
      rateButton.style.margin = "2px 0";
      rateButton.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
      rateButton.style.color = "white";
      rateButton.style.border = "1px solid gray";
      rateButton.style.cursor = "pointer";
      rateButton.style.textAlign = "left";

      if (rate === 1) {
        rateButton.style.backgroundColor = "rgba(0, 128, 255, 0.5)";
      }

      rateButton.addEventListener("click", () => {
        video.playbackRate = rate;
        // Update active state
        rates.forEach((r) => {
          const btn = SettingMenu.querySelector(
            `button[data-rate="${r}"]`
          ) as HTMLButtonElement;
          if (btn) {
            btn.style.backgroundColor =
              r === rate ? "rgba(0, 128, 255, 0.5)" : "rgba(0, 0, 0, 0.5)";
          }
        });
      });
      rateButton.setAttribute("data-rate", rate.toString());
      SettingMenu.appendChild(rateButton);
    });

    // Handle offset input changes
    offsetInput.addEventListener("change", () => {
      const offset = parseFloat(offsetInput.value);
      if (!isNaN(offset)) {
        local_offset = offset;
        localStorage.setItem(`local_offset:${live_id}`, offset.toString());
      }
    });

    // Toggle menu visibility
    playbackRateButton.addEventListener("click", () => {
      SettingMenu.style.display =
        SettingMenu.style.display === "none" ? "block" : "none";
      // if display is block, button background color should be red
      if (SettingMenu.style.display === "block") {
        playbackRateButton.style.backgroundColor = "rgba(0, 128, 255, 0.5)";
      } else {
        playbackRateButton.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
      }
    });

    // Close menu when clicking outside
    document.addEventListener("click", (e) => {
      if (
        !playbackRateButton.contains(e.target as Node) &&
        !SettingMenu.contains(e.target as Node)
      ) {
        SettingMenu.style.display = "none";
      }
    });

    shakaSpacer.appendChild(playbackRateButton);
    shakaSpacer.appendChild(SettingMenu);

    let danmu_statistics: { ts: number; count: number }[] = [];

    // create a danmu statistics select into shaka-spacer
    let statisticKey = "";
    const statisticKeyInput = document.createElement("input");
    statisticKeyInput.style.height = "30px";
    statisticKeyInput.style.width = "100px";
    statisticKeyInput.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
    statisticKeyInput.style.color = "white";
    statisticKeyInput.style.border = "1px solid gray";
    statisticKeyInput.style.padding = "0 10px";
    statisticKeyInput.style.boxSizing = "border-box";
    statisticKeyInput.style.fontSize = "1em";
    statisticKeyInput.style.right = "55px";
    statisticKeyInput.placeholder = "弹幕统计过滤";
    statisticKeyInput.style.position = "absolute";

    function update_statistics() {
      let counts = {};
      danmu_records.forEach((e) => {
        if (statisticKey != "" && !e.content.includes(statisticKey)) {
          return;
        }
        const timestamp = e.ts + local_offset * 1000 - global_offset * 1000;
        if (timestamp < 0) {
          return;
        }
        const timeSlot = timestamp - (timestamp % DANMU_STATISTIC_GAP);
        counts[timeSlot] = (counts[timeSlot] || 0) + 1;
      });
      danmu_statistics = [];
      for (let ts in counts) {
        danmu_statistics.push({ ts: parseInt(ts), count: counts[ts] });
      }
    }

    update_statistics();

    if (isLive()) {
      setInterval(async () => {
        update_statistics();
      }, 10 * 1000);
    }

    statisticKeyInput.addEventListener("change", () => {
      statisticKey = statisticKeyInput.value;
      update_statistics();
    });

    shakaSpacer.appendChild(statisticKeyInput);

    // shaka-spacer should be flex-direction: column
    shakaSpacer.style.flexDirection = "column";

    function isLive() {
      return player.isLive();
    }

    function get_total() {
      return player.seekRange().end;
    }
    // 加载区间数据
    loadRanges();

    // add keydown event listener for '[' and ']' to control range
    document.addEventListener("keydown", async (e) => {
      const target = e.target as HTMLInputElement;
      if (
        (target.tagName.toLowerCase() === "input" && target.type === "text") ||
        target.tagName.toLowerCase() === "textarea"
      ) {
        return;
      }
      switch (e.key) {
        case "[":
        case "【":
          e.preventDefault();
          {
            const currentTime = parseFloat(video.currentTime.toFixed(2));
            if (currentRangeIndex >= 0 && currentRangeIndex < ranges.length) {
              // 有选中区间：更新当前区间的开始时间
              ranges[currentRangeIndex].start = currentTime;
              // 如果结束时间小于开始时间，自动设置为视频结尾
              if (ranges[currentRangeIndex].end <= currentTime) {
                ranges[currentRangeIndex].end = get_total();
              }
            } else {
              // 没有选中区间：创建新区间并选中
              const newRange: Range = {
                start: currentTime,
                end: get_total(),
                activated: true, // 新建区间默认为激活
              };
              ranges.push(newRange);
              currentRangeIndex = ranges.length - 1;
            }
            saveRanges();
            console.log(
              "Range updated:",
              ranges[currentRangeIndex],
              "Total ranges:",
              ranges.length
            );
          }
          break;
        case "]":
        case "】":
          e.preventDefault();
          {
            const currentTime = parseFloat(video.currentTime.toFixed(2));
            if (currentRangeIndex >= 0 && currentRangeIndex < ranges.length) {
              // 有选中区间：更新当前区间的结束时间
              ranges[currentRangeIndex].end = currentTime;
              // 如果开始时间大于结束时间，自动设置为0
              if (ranges[currentRangeIndex].start >= currentTime) {
                ranges[currentRangeIndex].start = 0;
              }
            } else {
              // 没有选中区间：创建新区间并选中
              const newRange: Range = {
                start: 0,
                end: currentTime,
                activated: true, // 新建区间默认为激活
              };
              ranges.push(newRange);
              currentRangeIndex = ranges.length - 1;
            }
            saveRanges();
            console.log(
              "Range updated:",
              ranges[currentRangeIndex],
              "Total ranges:",
              ranges.length
            );
          }
          break;
        case "Enter":
          e.preventDefault();
          {
            // 合并重叠区间
            mergeOverlappingRanges();
            // 取消选中，进入创建模式
            currentRangeIndex = -1;
            saveRanges();
            console.log("Entered range creation mode (no range selected)");
          }
          break;
        case "n":
        case "N":
          e.preventDefault();
          {
            // 创建新区间，从当前播放位置开始
            const currentTime = parseFloat(video.currentTime.toFixed(2));
            const newRange: Range = {
              start: currentTime,
              end: get_total(),
              activated: true, // 新建区间默认为激活
            };
            ranges.push(newRange);
            currentRangeIndex = ranges.length - 1;
            saveRanges();
            console.log(
              "New range created:",
              newRange,
              "Total ranges:",
              ranges.length
            );
          }
          break;
        case "d":
        case "Delete":
          e.preventDefault();
          {
            if (currentRangeIndex >= 0 && currentRangeIndex < ranges.length) {
              ranges.splice(currentRangeIndex, 1);
              // 调整当前索引
              if (ranges.length === 0) {
                currentRangeIndex = -1;
              } else if (currentRangeIndex >= ranges.length) {
                currentRangeIndex = ranges.length - 1;
              }
              saveRanges();
              console.log("Range deleted, remaining:", ranges.length);
            }
          }
          break;
        case "Tab":
        case "t":
        case "T":
          e.preventDefault();
          {
            if (ranges.length > 0) {
              if (e.shiftKey) {
                // Shift+Tab or Shift+t: 切换到上一个区间
                currentRangeIndex =
                  currentRangeIndex <= 0
                    ? ranges.length - 1
                    : currentRangeIndex - 1;
              } else {
                // Tab or t: 切换到下一个区间
                currentRangeIndex = (currentRangeIndex + 1) % ranges.length;
              }
              saveRanges();
              const current = ranges[currentRangeIndex];
              console.log("Switched to range:", currentRangeIndex, current);
            }
          }
          break;
        case " ":
          e.preventDefault();
          if (e.repeat) {
            break;
          }
          if (video.paused) {
            video.play();
          } else {
            video.pause();
          }
          break;
        case "p":
          e.preventDefault();
          if (e.repeat) {
            break;
          }
          // dispatch event
          dispatch("markerAdd", {
            offset: video.currentTime,
            realtime: global_offset + video.currentTime,
          });
          break;
        case "ArrowLeft":
          e.preventDefault();
          video.currentTime -= 3;
          break;
        case "ArrowRight":
          e.preventDefault();
          video.currentTime += 3;
          break;
        case "q":
          e.preventDefault();
          {
            const current = getCurrentRange();
            if (current) {
              video.currentTime = current.start;
            } else {
              video.currentTime = 0;
            }
          }
          break;
        case "e":
          e.preventDefault();
          {
            const current = getCurrentRange();
            if (current) {
              video.currentTime = current.end;
            } else {
              video.currentTime = get_total();
            }
          }
          break;
        case "c":
          e.preventDefault();
          ranges = [];
          currentRangeIndex = -1;
          saveRanges();
          console.log("All ranges cleared");
          break;
        case "h":
          e.preventDefault();
          show_detail = !show_detail;
          break;
        case "Escape":
          e.preventDefault();
          resetZoom();
          break;
        case "g":
          e.preventDefault();
          {
            const current = getCurrentRange();
            if (current && current.start < current.end) {
              // 在跳转前先保存所有区间，确保数据不丢失
              saveRanges();
              zoomOnRange(
                focus_start + current.start,
                focus_start + current.end
              );
            }
          }
          break;
        case "a": // Add 'a' key for toggling activated status
          e.preventDefault();
          {
            const current = getCurrentRange();
            if (current) {
              current.activated = !current.activated;
              ranges = [...ranges]; // Trigger Svelte reactivity for array update
              saveRanges();
              console.log(
                `Range ${currentRangeIndex} activated status toggled to: ${current.activated}`
              );
            } else {
              console.log("No current range selected to toggle activation.");
            }
          }
          break;
      }
    });

    const seekbarContainer = selfSeekbar.querySelector(
      ".shaka-seek-bar-container.self-defined"
    ) as HTMLElement;

    // Add click listener to seekbar to toggle range activation
    seekbarContainer.addEventListener("click", (e) => {
      const rect = seekbarContainer.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickRatio = clickX / rect.width;
      const clickTime = clickRatio * get_total();

      for (const range of ranges) {
        if (clickTime >= range.start && clickTime <= range.end) {
          range.activated = !range.activated;
          // reassign ranges to trigger Svelte reactivity after mutation
          ranges = [...ranges];
          // save and redraw will be handled by the animation frame
          saveRanges();
          break; // only toggle the first one found
        }
      }
    });

    const statisticGraph = document.createElement(
      "canvas"
    ) as HTMLCanvasElement;
    statisticGraph.style.pointerEvents = "none";
    statisticGraph.style.position = "absolute";
    statisticGraph.style.bottom = "8px";
    statisticGraph.style.zIndex = "20";
    const canvas = statisticGraph.getContext("2d");
    seekbarContainer.appendChild(statisticGraph);

    // 创建当前区间高亮覆盖层
    const currentRangeHighlight = document.createElement("div");
    currentRangeHighlight.style.pointerEvents = "none";
    currentRangeHighlight.style.position = "absolute";
    currentRangeHighlight.style.top = "-2px";
    currentRangeHighlight.style.height = "calc(100% + 4px)";
    currentRangeHighlight.style.backgroundColor = "rgba(255, 215, 0, 0.6)"; // 金黄色，半透明
    currentRangeHighlight.style.border = "2px solid rgb(255, 165, 0)"; // 橙色边框
    currentRangeHighlight.style.borderRadius = "2px";
    currentRangeHighlight.style.zIndex = "25";
    currentRangeHighlight.style.display = "none";
    currentRangeHighlight.style.boxShadow = "0 0 8px rgba(255, 215, 0, 0.8)"; // 发光效果
    seekbarContainer.appendChild(currentRangeHighlight);

    // draw statistics
    function drawStatistics(points: { ts: number; count: number }[]) {
      if (points == undefined) {
        points = [];
      }
      // preprocess points
      let preprocessed = [];
      for (let i = 1; i < points.length; i++) {
        preprocessed.push(points[i - 1]);
        let gap = (points[i].ts - points[i - 1].ts) / 1000;
        if (gap > DANMU_STATISTIC_GAP) {
          // add zero point to fill gap
          let cnt = 1;
          while (gap > DANMU_STATISTIC_GAP) {
            preprocessed.push({
              ts: points[i - 1].ts + cnt * DANMU_STATISTIC_GAP * 1000,
              count: 0,
            });
            cnt += 1;
            gap -= DANMU_STATISTIC_GAP;
          }
        }
      }
      if (points.length > 0) {
        preprocessed.push(points[points.length - 1]);
      }
      const scale = window.devicePixelRatio || 1;
      statisticGraph.width = seekbarContainer.clientWidth * scale;
      statisticGraph.height = 16 * scale;
      statisticGraph.style.width = `${seekbarContainer.clientWidth}px`;
      statisticGraph.style.height = "16px";
      const canvasHeight = statisticGraph.height;
      const canvasWidth = statisticGraph.width;
      // find value range
      const minValue = 0;
      let maxValue = 0;
      if (preprocessed.length > 0) {
        const counts = preprocessed
          .map((v) => v.count)
          .filter((c) => isFinite(c));
        if (counts.length > 0) {
          // Use reduce instead of spread operator to avoid stack overflow
          maxValue = counts.reduce((max, current) => Math.max(max, current), 0);
        }
      }
      const duration = get_total() * 1000;
      canvas.clearRect(0, 0, canvasWidth, canvasHeight);
      if (preprocessed.length > 0) {
        canvas.beginPath();
        const x = (preprocessed[0].ts / duration) * canvasWidth;
        const y =
          (1 - (preprocessed[0].count - minValue) / (maxValue - minValue)) *
          canvasHeight;
        canvas.moveTo(x, y);
        for (let i = 0; i < preprocessed.length; i++) {
          const x = (preprocessed[i].ts / duration) * canvasWidth;
          const y =
            (1 - (preprocessed[i].count - minValue) / (maxValue - minValue)) *
            canvasHeight;
          canvas.lineTo(x, y);
          if (i == preprocessed.length - 1) {
            canvas.lineTo(x, canvasHeight);
          }
        }
        canvas.strokeStyle = "rgba(245, 166, 39, 0.5)";
        canvas.stroke();
        canvas.lineTo(x, canvasHeight);
        canvas.closePath();
        canvas.fillStyle = "rgba(245, 166, 39, 0.5)";
        canvas.fill();
      }
    }

    function updateSeekbar() {
      const total = get_total();

      // 更新当前区间高亮覆盖层
      const currentRange =
        currentRangeIndex >= 0 && currentRangeIndex < ranges.length
          ? ranges[currentRangeIndex]
          : null;

      if (currentRange && total > 0) {
        const rangeStart = currentRange.start / total;
        const rangeEnd = currentRange.end / total;
        const containerWidth = seekbarContainer.clientWidth;
        const left = rangeStart * containerWidth;
        const width = (rangeEnd - rangeStart) * containerWidth;

        currentRangeHighlight.style.left = `${left}px`;
        currentRangeHighlight.style.width = `${width}px`;
        currentRangeHighlight.style.display = "block";
      } else {
        currentRangeHighlight.style.display = "none";
      }

      // 构建多区间渐变背景
      if (ranges.length === 0) {
        // 没有区间时，显示默认背景
        seekbarContainer.style.background = "rgba(255, 255, 255, 0.4)";
      } else {
        // 按时间顺序排序区间
        const sortedRanges = [...ranges].sort((a, b) => a.start - b.start);
        const gradientStops: string[] = [];
        let lastPos = 0;

        for (let i = 0; i < sortedRanges.length; i++) {
          const range = sortedRanges[i];
          // 计算区间在当前 focus 范围内的可见部分
          // range.start 和 range.end 是相对于当前 focus_start 的时间
          const visibleStart = Math.max(0, range.start);
          const visibleEnd = Math.min(total, range.end);

          // 如果区间在当前 focus 范围内有可见部分，才显示
          if (visibleEnd > visibleStart) {
            const rangeStart = visibleStart / total;
            const rangeEnd = visibleEnd / total;

            // 添加区间前的背景
            if (rangeStart > lastPos) {
              gradientStops.push(`rgba(255, 255, 255, 0.4) ${lastPos * 100}%`);
              gradientStops.push(
                `rgba(255, 255, 255, 0.4) ${rangeStart * 100}%`
              );
            }

            // 'activated' 默认为 true
            const rangeColor =
              range.activated !== false
                ? "rgb(0, 200, 0)"
                : "rgb(80, 80, 80)"; // Active: green, Inactive: gray
            gradientStops.push(`${rangeColor} ${rangeStart * 100}%`);
            gradientStops.push(`${rangeColor} ${rangeEnd * 100}%`);

            lastPos = rangeEnd;
          }
        }

        // 添加最后一个区间后的背景
        if (lastPos < 1) {
          gradientStops.push(`rgba(255, 255, 255, 0.4) ${lastPos * 100}%`);
          gradientStops.push(`rgba(255, 255, 255, 0.4) 100%`);
        }

        seekbarContainer.style.background = `linear-gradient(to right, ${gradientStops.join(
          ", "
        )})`;
      }
      // render markers in shaka-ad-markers
      const adMarkers = document.querySelector(
        ".shaka-ad-markers"
      ) as HTMLElement;
      if (adMarkers) {
        // clean previous markers
        adMarkers.innerHTML = "";
        for (const marker of markers) {
          const markerElement = document.createElement("div");
          markerElement.style.position = "absolute";
          markerElement.style.width = "6px";
          markerElement.style.height = "7px";
          markerElement.style.backgroundColor = "#93A8AC";
          markerElement.style.left = `calc(${(marker.offset / total) * 100}% - 3px)`;
          markerElement.style.top = "-12px";
          markerElement.style.zIndex = "30";
          // little triangle on the bottom
          const triangle = document.createElement("div");
          triangle.style.width = "0";
          triangle.style.height = "0";
          triangle.style.borderLeft = "3px solid transparent";
          triangle.style.borderRight = "3px solid transparent";
          triangle.style.borderTop = "4px solid #93A8AC";
          triangle.style.position = "absolute";
          triangle.style.top = "7px";
          triangle.style.left = "0";
          markerElement.appendChild(triangle);
          adMarkers.appendChild(markerElement);
        }
        drawStatistics(danmu_statistics);
      }
      requestAnimationFrame(updateSeekbar);
    }
    requestAnimationFrame(updateSeekbar);
  }

  // receive tauri emit
  document.addEventListener("shaka-ui-loaded", init);

  // set body background color to black
  document.body.style.backgroundColor = "black";

  // Add blur event listener to close menus
  window.addEventListener("blur", () => {
    show_list = false;
    show_export = false;
  });

  async function exportDanmu(ass: boolean) {
    console.log("Export danmus");
    const assContent = (await invoke("export_danmu", {
      options: {
        platform: platform,
        roomId: room_id,
        liveId: live_id,
        x: Math.floor(focus_start + start),
        y: Math.floor(focus_start + end),
        offset: global_offset,
        ass: ass,
      },
    })) as string;

    let file_name = `danmu_${room_id}_${live_id}.${ass ? "ass" : "txt"}`;
    if (TAURI_ENV) {
      const path = await save({
        title: "导出弹幕",
        defaultPath: file_name,
      });
      if (!path) return;
      await invoke("export_to_file", { fileName: path, content: assContent });
    } else {
      const a = document.createElement("a");
      a.href =
        "data:text/plain;charset=utf-8," + encodeURIComponent(assContent);
      a.download = file_name;
      a.click();
    }
  }
</script>

<section id="wrap">
  <div
    class="youtube-theme"
    data-shaka-player-container
    style="width: 100%; height: 100vh;"
  >
    <!-- svelte-ignore a11y-media-has-caption -->
    <video autoplay data-shaka-player id="video" disablepictureinpicture
    ></video>
  </div>
</section>
<div id="overlay">
  <p>
    快捷键说明
    <kbd>h</kbd>展开
  </p>
  {#if show_detail}
    <span>
      <p><kbd>Esc</kbd>返回直播/录播</p>
      <p><kbd>Space</kbd>播放/暂停</p>
      <p><kbd>[</kbd>设定当前区间开始（无选中时创建新区间）</p>
      <p><kbd>]</kbd>设定当前区间结束（无选中时创建新区间）</p>
      <p><kbd>Enter</kbd>取消选中，进入创建模式</p>
      <p><kbd>n</kbd>创建新区间</p>
      <p><kbd>d</kbd>删除当前区间</p>
      <p><kbd>Tab</kbd>/<kbd>t</kbd>切换到下一个区间</p>
      <p><kbd>Shift+Tab</kbd>/<kbd>Shift+t</kbd>切换到上一个区间</p>
      <p><kbd>g</kbd>预览当前区间片段</p>
      <p><kbd>a</kbd>切换当前区间激活状态</p>
      <p><kbd>q</kbd>跳转到当前区间开始</p>
      <p><kbd>e</kbd>跳转到当前区间结束</p>
      <p><kbd>←</kbd>前进</p>
      <p><kbd>→</kbd>后退</p>
      <p><kbd>c</kbd>清除所有区间</p>
      <p><kbd>p</kbd>创建标记</p>
    </span>
  {/if}
</div>
<div id="shortcuts">
  <button id="shortcut-btn">
    <GridOutline />
  </button>
  <ul class="shortcut-list">
    {#each recorders as recorder}
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <li
        class="shortcut"
        on:click={() => {
          go_to(
            recorder.room_info.platform,
            recorder.room_info.room_id,
            recorder.live_id
          );
        }}
      >
        <SortHorizontalOutline />[{recorder.user_info.user_name}]{recorder
          .room_info.room_title}
      </li>
    {/each}
    {#if recorders.length == 0}
      <p>没有其它正在直播的房间</p>
    {/if}
  </ul>
</div>

<div id="export">
  <button id="export-btn">
    <FileExportOutline />
  </button>
  <ul class="export-list">
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <li
      class="export-item"
      on:click={() => {
        exportDanmu(false);
      }}
    >
      导出弹幕为 TXT
    </li>
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <li
      class="export-item"
      on:click={() => {
        exportDanmu(true);
      }}
    >
      导出弹幕为 ASS
    </li>
  </ul>
</div>

<style>
  video {
    width: 100%;
    height: 100%;
  }

  p {
    margin: 0;
  }

  kbd {
    border: 1px solid white;
    padding: 0 0.2em;
    border-radius: 0.2em;
    margin: 4px;
  }

  #overlay {
    position: absolute;
    top: 8px;
    left: 8px;
    border-radius: 6px;
    padding: 8px;
    flex-direction: column;
    display: flex;
    background-color: rgba(0, 0, 0, 0.5);
    color: white;
    font-size: 0.8em;
    pointer-events: none;
  }

  #shortcuts {
    position: absolute;
    top: 8px;
    right: 52px;
    flex-direction: column;
    display: flex;
    align-items: end;
    color: white;
    font-size: 0.8em;
    z-index: 501;
  }

  #shortcut-btn {
    width: 36px;
    padding: 8px;
    margin-bottom: 4px;
    border-radius: 4px;
    cursor: pointer;
    background-color: rgba(0, 0, 0, 0.5);
  }

  #shortcut-btn:hover {
    background-color: rgba(255, 255, 255, 0.3);
  }

  .shortcut-list {
    border-radius: 4px;
    padding: 8px;
    background-color: rgba(0, 0, 0, 0.5);
    display: none;
    position: absolute;
    top: 100%;
    right: 0;
    min-width: 200px;
  }

  #shortcuts:hover .shortcut-list {
    display: block;
  }

  .shortcut {
    display: flex;
    flex-direction: row;
    cursor: pointer;
  }

  .shortcut:hover {
    text-decoration: underline;
  }

  #export {
    position: absolute;
    top: 8px;
    right: 8px;
    flex-direction: column;
    display: flex;
    align-items: end;
    color: white;
    font-size: 0.8em;
    z-index: 501;
  }

  #export-btn {
    width: 36px;
    padding: 8px;
    margin-bottom: 4px;
    border-radius: 4px;
    cursor: pointer;
    background-color: rgba(0, 0, 0, 0.5);
  }

  #export-btn:hover {
    background-color: rgba(255, 255, 255, 0.3);
  }

  .export-list {
    border-radius: 4px;
    padding: 8px;
    background-color: rgba(0, 0, 0, 0.5);
    display: none;
    position: absolute;
    top: 100%;
    right: 0;
    min-width: 150px;
  }

  #export:hover .export-list {
    display: block;
  }

  .export-item {
    display: flex;
    flex-direction: row;
    cursor: pointer;
    padding: 4px 8px;
  }

  .export-item:hover {
    background-color: rgba(255, 255, 255, 0.1);
  }
</style>
