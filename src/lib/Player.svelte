<script lang="ts" context="module">
  declare const shaka: any;
</script>

<script lang="ts">
  import { invoke, TAURI_ENV, ENDPOINT, listen } from "../lib/invoker";
  import type { AccountInfo } from "./db";
  import type { Marker, RecorderList, RecorderInfo } from "./interface";

  import { createEventDispatcher } from "svelte";
  import {
    GridOutline,
    SortHorizontalOutline,
    FileExportOutline,
  } from "flowbite-svelte-icons";
  import { save } from "@tauri-apps/plugin-dialog";
  const dispatch = createEventDispatcher();

  interface DanmuEntry {
    ts: number;
    content: string;
  }

  export let platform: string;
  export let room_id: number;
  export let live_id: string;
  export let start = 0;
  export let end = 0;
  export let global_offset = 0;
  export let focus_start = 0;
  export let focus_end = 0;
  export let markers: Marker[] = [];
  export function seek(offset: number) {
    video.currentTime = offset;
  }
  let video: HTMLVideoElement;
  let show_detail = false;
  let show_list = false;
  let show_export = false;
  let recorders: RecorderInfo[] = [];

  // save start and end to localStorage
  function saveStartEnd() {
    localStorage.setItem(`${live_id}_start`, (start + focus_start).toString());
    localStorage.setItem(`${live_id}_end`, (end + focus_start).toString());
    console.log("Saved start and end", start + focus_start, end + focus_start);
  }

  async function loadGlobalOffset(url: string) {
    const response = await fetch(url);
    const text = await response.text();
    const offsetRegex = /#EXT-X-OFFSET:(\d+)/;
    const match = text.match(offsetRegex);
    if (match && match[1]) {
      global_offset = parseInt(match[1], 10);
    } else {
      console.warn("No #EXT-X-OFFSET found");
    }
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
            const offsetRegex = /#EXT-X-OFFSET:(\d+)/;
            const match = m3u8Content.match(offsetRegex);

            if (match && match[1]) {
              global_offset = parseInt(match[1], 10);
            } else {
              console.warn("No #EXT-X-OFFSET found");
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
          console.error("Network error:", error);
          reject(
            new shaka.util.Error(
              shaka.util.Error.Severity.CRITICAL,
              shaka.util.Error.Category.NETWORK,
              shaka.util.Error.Code.OPERATION_ABORTED,
              error.message || "Network request failed"
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
    shaka.net.NetworkingEngine.registerScheme("http", tauriNetworkPlugin);
    shaka.net.NetworkingEngine.registerScheme("https", tauriNetworkPlugin);
  }

  async function update_stream_list() {
    recorders = (
      (await invoke("get_recorder_list")) as RecorderList
    ).recorders.filter((r) => r.live_status && r.room_id != room_id);
  }

  function go_to(platform: string, room_id: number, live_id: string) {
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
      const url = `${ENDPOINT}/hls/${platform}/${room_id}/${live_id}/master.m3u8?start=${focus_start}&end=${focus_end}`;
      if (!TAURI_ENV) {
        await loadGlobalOffset(url);
      }
      await player.load(url);
      // This runs if the asynchronous load is successful.
      console.log("The video has now been loaded!");
    } catch (error) {
      console.error("Error code", error.code, "object", error);
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
          <div class="shaka-seek-bar-container self-defined" style="background-color: gray; margin: 4px 10px 4px 10px;">
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
    let danmu_records: DanmuEntry[] = (await invoke("get_danmu_record", {
      roomId: room_id,
      liveId: live_id,
      platform: platform,
    })) as DanmuEntry[];

    console.log("danmu loaded:", danmu_records.length);

    let ts = parseInt(live_id);

    if (platform == "bilibili") {
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
          (video.currentTime + global_offset + ts + focus_start) * 1000
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
              const uid = parseInt(accountSelect.value);
              await invoke("send_danmaku", {
                uid,
                roomId: room_id,
                ts,
                message: value,
              });
              danmakuInput.value = "";
            }
          }
        });

        shakaSpacer.appendChild(accountSelect);
        shakaSpacer.appendChild(danmakuInput);

        // listen to danmaku event
        await listen("danmu:" + room_id, (event: { payload: DanmuEntry }) => {
          // if not enabled or playback is not keep up with live, ignore the danmaku
          if (!danmu_enabled || get_total() - video.currentTime > 5) {
            danmu_records.push(event.payload);
            return;
          }
          if (Object.keys(danmu_displayed).length > 1000) {
            danmu_displayed = {};
          }
          danmu_displayed[event.payload.ts] = true;
          danmu_records.push(event.payload);
          danmu_handler(event.payload.content);
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
    }

    // create a playback rate select to of shaka-spacer
    const playbackRateSelect = document.createElement("select");
    playbackRateSelect.style.height = "30px";
    playbackRateSelect.style.minWidth = "60px";
    playbackRateSelect.style.backgroundColor = "rgba(0, 0, 0, 0.5)";
    playbackRateSelect.style.color = "white";
    playbackRateSelect.style.border = "1px solid gray";
    playbackRateSelect.style.padding = "0 10px";
    playbackRateSelect.style.boxSizing = "border-box";
    playbackRateSelect.style.fontSize = "1em";
    playbackRateSelect.style.right = "10px";
    playbackRateSelect.style.position = "absolute";
    playbackRateSelect.innerHTML = `
      <option value="0.5">0.5x</option>
      <option value="1">1x</option>
      <option value="1.5">1.5x</option>
      <option value="2">2x</option>
      <option value="5">5x</option>
    `;
    // default playback rate is 1
    playbackRateSelect.value = "1";
    playbackRateSelect.addEventListener("change", () => {
      const rate = parseFloat(playbackRateSelect.value);
      video.playbackRate = rate;
    });

    shakaSpacer.appendChild(playbackRateSelect);

    let danmu_statistics: { ts: number; count: number }[] = [];

    if (platform == "bilibili") {
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
      statisticKeyInput.style.right = "75px";
      statisticKeyInput.placeholder = "弹幕统计过滤";
      statisticKeyInput.style.position = "absolute";

      function update_statistics() {
        let counts = {};
        danmu_records.forEach((e) => {
          if (statisticKey != "" && !e.content.includes(statisticKey)) {
            return;
          }
          const timeSlot = Math.floor(e.ts / 10000) * 10000; // 将时间戳向下取整到10秒
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
    }

    // shaka-spacer should be flex-direction: column
    shakaSpacer.style.flexDirection = "column";

    function isLive() {
      return player.isLive();
    }

    function get_total() {
      return player.seekRange().end;
    }
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
          e.preventDefault();
          start = parseFloat(video.currentTime.toFixed(2));
          if (end < start) {
            end = get_total();
          }

          saveStartEnd();
          console.log(start, end);
          break;
        case "【":
          e.preventDefault();
          start = parseFloat(video.currentTime.toFixed(2));
          if (end < start) {
            end = get_total();
          }
          saveStartEnd();
          console.log(start, end);
          break;
        case "]":
          e.preventDefault();
          end = parseFloat(video.currentTime.toFixed(2));
          if (start > end) {
            start = 0;
          }
          saveStartEnd();
          console.log(start, end);
          break;
        case "】":
          e.preventDefault();
          end = parseFloat(video.currentTime.toFixed(2));
          if (start > end) {
            start = 0;
          }
          saveStartEnd();
          console.log(start, end);
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
            realtime: ts + video.currentTime,
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
          video.currentTime = start;
          break;
        case "e":
          e.preventDefault();
          if (end == 0) {
            video.currentTime = get_total();
          } else {
            video.currentTime = end;
          }
          break;
        case "c":
          e.preventDefault();
          start = 0;
          end = 0;
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
          if ((start == 0 && end == 0) || start > end) {
            break;
          }
          zoomOnRange(focus_start + start, focus_start + end);
          break;
      }
    });

    const seekbarContainer = selfSeekbar.querySelector(
      ".shaka-seek-bar-container.self-defined"
    ) as HTMLElement;

    const statisticGraph = document.createElement(
      "canvas"
    ) as HTMLCanvasElement;
    statisticGraph.style.pointerEvents = "none";
    statisticGraph.style.position = "absolute";
    statisticGraph.style.bottom = "11px";
    statisticGraph.style.zIndex = "20";
    const canvas = statisticGraph.getContext("2d");
    seekbarContainer.appendChild(statisticGraph);

    // draw statistics
    function drawStatistics(points: { ts: number; count: number }[]) {
      if (player.getPresentationStartTimeAsDate() == null) {
        return;
      }
      if (points == undefined) {
        points = [];
      }
      // preprocess points
      let preprocessed = [];
      for (let i = 1; i < points.length; i++) {
        preprocessed.push(points[i - 1]);
        let gap = (points[i].ts - points[i - 1].ts) / 1000;
        if (gap > 10) {
          // add zero point to fill gap
          let cnt = 1;
          while (gap > 10) {
            preprocessed.push({
              ts: points[i - 1].ts + cnt * 10 * 1000,
              count: 0,
            });
            cnt += 1;
            gap -= 10;
          }
        }
      }
      if (points.length > 0) {
        preprocessed.push(points[points.length - 1]);
      }
      const scale = window.devicePixelRatio || 1;
      statisticGraph.width = seekbarContainer.clientWidth * scale;
      statisticGraph.height = 30 * scale;
      statisticGraph.style.width = `${seekbarContainer.clientWidth}px`;
      statisticGraph.style.height = "30px";
      const canvasHeight = statisticGraph.height;
      const canvasWidth = statisticGraph.width;
      // find value range
      const minValue = 0;
      const maxValue = Math.max(...preprocessed.map((v) => v.count));
      const beginTime = player.getPresentationStartTimeAsDate().getTime();
      const duration = get_total() * 1000;
      canvas.clearRect(0, 0, canvasWidth, canvasHeight);
      if (preprocessed.length > 0) {
        canvas.beginPath();
        const x = ((preprocessed[0].ts - beginTime) / duration) * canvasWidth;
        const y =
          (1 - (preprocessed[0].count - minValue) / (maxValue - minValue)) *
          canvasHeight;
        canvas.moveTo(x, y);
        for (let i = 0; i < preprocessed.length; i++) {
          const x = ((preprocessed[i].ts - beginTime) / duration) * canvasWidth;
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
      const first_point = start / total;
      const second_point = end / total;
      // set background color for self-defined seekbar between first_point and second_point using linear-gradient
      seekbarContainer.style.background = `linear-gradient(to right, rgba(255, 255, 255, 0.4) ${
        first_point * 100
      }%, rgb(0, 255, 0) ${first_point * 100}%, rgb(0, 255, 0) ${
        second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${
        second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${
        first_point * 100
      }%, rgba(255, 255, 255, 0.2) ${first_point * 100}%)`;
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
        offset: global_offset + parseInt(live_id),
        ass: ass,
      },
    })) as string;

    let file_name = `danmu_${room_id}_${live_id}.${ass ? "ass" : "txt"}`;
    const path = await save({
      title: "导出弹幕",
      defaultPath: file_name,
    });
    if (!path) return;
    await invoke("export_to_file", { fileName: path, content: assContent });
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
      <p><kbd>[</kbd>设定选区开始</p>
      <p><kbd>]</kbd>设定选区结束</p>
      <p><kbd>g</kbd>预览选区片段</p>
      <p><kbd>q</kbd>跳转到选区开始</p>
      <p><kbd>e</kbd>跳转到选区结束</p>
      <p><kbd>←</kbd>前进</p>
      <p><kbd>→</kbd>后退</p>
      <p><kbd>c</kbd>清除选区</p>
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
          go_to(recorder.platform, recorder.room_id, recorder.current_live_id);
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
