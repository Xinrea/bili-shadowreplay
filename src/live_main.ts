import { invoke } from "@tauri-apps/api/tauri";
import { appWindow } from "@tauri-apps/api/window";
const urlParams = new URLSearchParams(window.location.search);
const port = urlParams.get("port");
const room_id = urlParams.get("room_id");
console.log(port, room_id);
let x_offset = 0;
let y_offset = 0;
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
    }
  };
  ui.configure(config);
  // Attach player and UI to the window to make it easy to access in the JS console.
  (window as any).player = player;
  (window as any).ui = ui;
  try {
    await player.load(`http://127.0.0.1:${port}/${room_id}/playlist.m3u8`);
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
  document.querySelector(
    ".shaka-back-to-overflow-button .material-icons-round"
  ).innerHTML = "arrow_back_ios_new";

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
  // add keydown event listener for '[' and ']' to control range
  document.addEventListener("keydown", (e) => {
    const start = player.getPresentationStartTimeAsDate();
    switch (e.key) {
      case "[":
        x_offset = (player.getPlayheadTimeAsDate() - start) / 1000;
        if (y_offset < x_offset) {
          y_offset = (Date.now() - start) / 1000;
        }
        break;
      case "]":
        y_offset = (player.getPlayheadTimeAsDate() - start) / 1000;
        if (x_offset > y_offset) {
          x_offset = 0;
        }
        break;
      case "Enter":
        if (y_offset > 0) {
          invoke("clip_range", { roomId: parseInt(room_id), x: x_offset, y: y_offset, upload: e.altKey });
        }
        break;
      case " ":
        if (video.paused) {
          video.play();
        } else {
          video.pause();
        }
        break;
      case "m":
        video.muted = !video.muted;
        break;
      case "ArrowLeft":
        video.currentTime -= 1;
        break;
      case "ArrowRight":
        video.currentTime += 1;
        break;
      case "q":
        video.currentTime = x_offset;
        break;
      case "e":
        if (y_offset == 0) {
          video.currentTime = (Date.now() - start) / 1000;
        } else {
          video.currentTime = y_offset;
        }
        break;
      case 'c':
        x_offset = 0;
        y_offset = 0;
        break
      case 'Escape':
        // close window
        appWindow.close();
        break;
    }
  });
  setInterval(() => {
    const start = player.getPresentationStartTimeAsDate();
    const total = (Date.now() - start) / 1000;
    const first_point = x_offset / total;
    const second_point = y_offset / total;
    // set background color for self-defined seekbar between first_point and second_point using linear-gradient
    // example: linear-gradient(to right, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.63019%, rgba(255, 255, 255, 0.4) 28.63019%, rgba(255, 255, 255, 0.4) 29.285618%, rgba(255, 255, 255, 0.2) 29.285618%)
    const seekbarContainer = selfSeekbar.querySelector(
      ".shaka-seek-bar-container.self-defined"
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
  }, 100);
}
// receive tauri emit
document.addEventListener("shaka-ui-loaded", init);
