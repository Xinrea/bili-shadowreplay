<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type { AccountInfo, AccountItem } from "./db";

  interface DanmuEntry {
    ts: number;
    content: string;
  }

  export let port;
  export let room_id;
  export let ts;
  export let start = 0;
  export let end = 0;
  let show_detail = false;
  let global_offset = 0;

  // TODO get custom tag from shaka player instead of manual parsing
  async function meta_parse() {
    fetch(`http://127.0.0.1:${port}/${room_id}/${ts}/playlist.m3u8`)
      .then((response) => response.text())
      .then((m3u8Content) => {
        const offsetRegex = /#EXT-X-OFFSET:(\d+)/;
        const match = m3u8Content.match(offsetRegex);

        if (match && match[1]) {
          global_offset = parseInt(match[1], 10);
        } else {
          console.warn("No #EXT-X-OFFSET found");
        }
      })
      .catch((error) => {
        console.error("Error fetching M3U8 file:", error);
      });
  }

  async function init() {
    const video = document.getElementById("video") as HTMLVideoElement;
    const ui = video["ui"];
    const controls = ui.getControls();
    const player = controls.getPlayer();

    const config = {
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

    player.addEventListener("ended", async () => {
      location.reload();
    });
    player.addEventListener("manifestloaded", (event) => {
      console.log("Manifest loaded:", event);
    });

    try {
      await player.load(
        `http://127.0.0.1:${port}/${room_id}/${ts}/playlist.m3u8`,
      );
      // This runs if the asynchronous load is successful.
      console.log("The video has now been loaded!");
    } catch (error) {
      console.error("Error code", error.code, "object", error);
      if (error.code == 3000) {
        // reload
        location.reload();
      }
    }

    document.getElementsByClassName("shaka-overflow-menu-button")[0].remove();
    document.getElementsByClassName("shaka-fullscreen-button")[0].remove();
    // add self-defined element in shaka-bottom-controls.shaka-no-propagation (second seekbar)
    const shakaBottomControls = document.querySelector(
      ".shaka-bottom-controls.shaka-no-propagation",
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
      ts: ts,
    })) as DanmuEntry[];

    console.log("danmu loaded:", danmu_records.length);

    // history danmaku sender
    setInterval(() => {
      if (video.paused) {
        return;
      }
      if (danmu_records.length == 0) {
        return;
      }
      // using live source
      if (isLive() && get_total() - video.currentTime <= 5) {
        return;
      }
      if (!isLive()) {
        const cur = (video.currentTime + global_offset / 1000 + ts) * 1000;
        console.log(video.currentTime, new Date(cur).toString());
        let danmus = danmu_records.filter(
          (v) => v.ts >= cur - 1000 && v.ts < cur,
        );
        danmus.forEach((v) => danmu_handler(v.content));
      } else {
        const cur = player.getPlayheadTimeAsDate();
        let danmus = danmu_records.filter(
          (v) => v.ts >= cur - 1000 && v.ts < cur,
        );
        danmus.forEach((v) => danmu_handler(v.content));
      }
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
        const option = document.createElement("option");
        option.value = account.uid.toString();
        option.text = account.name;
        accountSelect.appendChild(option);
      });
      // add a danmaku send input
      const danmakuInput = document.createElement("input");
      danmakuInput.type = "text";
      danmakuInput.placeholder = "回车发送弹幕";
      danmakuInput.style.width = "50%";
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
      listen("danmu:" + room_id, (event: { payload: DanmuEntry }) => {
        // add into records
        danmu_records.push(event.payload);
        // if not enabled or playback is not keep up with live, ignore the danmaku
        if (!danmu_enabled || get_total() - video.currentTime > 5) {
          return;
        }
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
      let topPosition;
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
          start = parseFloat(video.currentTime.toFixed(2));
          if (end < start) {
            end = get_total();
          }
          console.log(start, end);
          break;
        case "]":
          end = parseFloat(video.currentTime.toFixed(2));
          if (start > end) {
            start = 0;
          }
          console.log(start, end);
          break;
        case " ":
          if (e.repeat) {
            break;
          }
          if (video.paused) {
            video.play();
          } else {
            video.pause();
          }
          break;
        case "m":
          if (e.repeat) {
            break;
          }
          video.muted = !video.muted;
          break;
        case "ArrowLeft":
          video.currentTime -= 3;
          break;
        case "ArrowRight":
          video.currentTime += 3;
          break;
        case "q":
          video.currentTime = start;
          break;
        case "e":
          if (end == 0) {
            video.currentTime = get_total();
          } else {
            video.currentTime = end;
          }
          break;
        case "c":
          start = 0;
          end = 0;
          break;
        case "h":
          show_detail = !show_detail;
          break;
      }
    });

    function updateSeekbar() {
      const total = get_total();
      const first_point = start / total;
      const second_point = end / total;
      // set background color for self-defined seekbar between first_point and second_point using linear-gradient
      const seekbarContainer = selfSeekbar.querySelector(
        ".shaka-seek-bar-container.self-defined",
      ) as HTMLElement;
      seekbarContainer.style.background = `linear-gradient(to right, rgba(255, 255, 255, 0.4) ${
        first_point * 100
      }%, rgb(0, 255, 0) ${first_point * 100}%, rgb(0, 255, 0) ${
        second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${
        second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${
        first_point * 100
      }%, rgba(255, 255, 255, 0.2) ${first_point * 100}%)`;
      requestAnimationFrame(updateSeekbar);
    }
    requestAnimationFrame(updateSeekbar);
  }

  meta_parse();

  // receive tauri emit
  document.addEventListener("shaka-ui-loaded", init);

  // set body background color to black
  document.body.style.backgroundColor = "black";
</script>

<section id="wrap">
  <div
    class="youtube-theme"
    data-shaka-player-container
    style="width: 100%; height: 100vh;"
  >
    <!-- svelte-ignore a11y-media-has-caption -->
    <video autoplay data-shaka-player id="video"></video>
  </div>
</section>
<div id="overlay">
  <p>
    快捷键说明
    <kbd>h</kbd>展开
  </p>
  {#if show_detail}
    <span>
      <p><kbd>Esc</kbd>关闭窗口</p>
      <p><kbd>Space</kbd>播放/暂停</p>
      <p><kbd>[</kbd>设定选区开始</p>
      <p><kbd>]</kbd>设定选区结束</p>
      <p><kbd>q</kbd>跳转到选区开始</p>
      <p><kbd>e</kbd>跳转到选区结束</p>
      <p><kbd>←</kbd>前进</p>
      <p><kbd>→</kbd>后退</p>
      <p><kbd>c</kbd>清除选区</p>
      <p><kbd>m</kbd>静音</p>
    </span>
  {/if}
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
    position: fixed;
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
</style>
