import { invoke } from "@tauri-apps/api/tauri";
import { appWindow } from "@tauri-apps/api/window";
const urlParams = new URLSearchParams(window.location.search);
const port = urlParams.get("port");
const room_id = urlParams.get("room_id");
const ts = parseInt(urlParams.get("ts"));

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
    await player.load(`http://127.0.0.1:${port}/${room_id}/${ts}/playlist.m3u8`);
    // This runs if the asynchronous load is successful.
    console.log("The video has now been loaded!");
  } catch (error) {
    console.error("Error code", error.code, "object", error);
    if (error.code == 3000) {
      // reload
      location.reload();
    }
  }
  player.addEventListener('ended', async () => {
    location.reload();
  })

  function generateCover() {
    var w = video.videoWidth;
    var h = video.videoHeight;
    var canvas = document.createElement('canvas');
    canvas.width = 1280;
    canvas.height = 720;
    var context = canvas.getContext('2d');
    context.drawImage(video, 0, 0, w, h, 0, 0, 1280, 720);
    return canvas.toDataURL();
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

  function get_total() {
    let total = video.duration;
    if (total == Infinity || total >= 4294967296) {
      total = (Date.now() - player.getPresentationStartTimeAsDate()) / 1000;
    }
    return total;
  }
  // add keydown event listener for '[' and ']' to control range
  document.addEventListener("keydown", async (e) => {
    switch (e.key) {
      case "[":
        x_offset = video.currentTime;
        if (y_offset < x_offset) {
          y_offset = get_total();
        }
        console.log(x_offset, y_offset);
        break;
      case "]":
        y_offset = video.currentTime;
        if (x_offset > y_offset) {
          x_offset = 0;
        }
        console.log(x_offset, y_offset);
        break;
      case "Enter":
        if (y_offset > 0) {
          appWindow.setTitle(`[${room_id}]${ts} 切片中···`);
          const video_file = await invoke("clip_range", { roomId: parseInt(room_id), ts: ts, x: x_offset, y: y_offset });
          appWindow.setTitle(`[${room_id}]${ts} 切片完成`);
          console.log("video file generatd:", video_file);
          if (e.altKey) {
            const cover = generateCover();
            invoke("prepare_upload", { roomId: parseInt(room_id), file: video_file, cover: cover }).catch(e => {
              console.error(e);
            })
          }
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
          video.currentTime = get_total();
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
    const total = get_total();
    const first_point = x_offset / total;
    const second_point = y_offset / total;
    // set background color for self-defined seekbar between first_point and second_point using linear-gradient
    // example: linear-gradient(to right, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.63019%, rgba(255, 255, 255, 0.4) 28.63019%, rgba(255, 255, 255, 0.4) 29.285618%, rgba(255, 255, 255, 0.2) 29.285618%)
    const seekbarContainer = selfSeekbar.querySelector(
      ".shaka-seek-bar-container.self-defined"
    ) as HTMLElement;
    seekbarContainer.style.background = `linear-gradient(to right, rgba(255, 255, 255, 0.4) ${first_point * 100
      }%, rgb(0, 255, 0) ${first_point * 100}%, rgb(0, 255, 0) ${second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${second_point * 100
      }%, rgba(255, 255, 255, 0.4) ${first_point * 100
      }%, rgba(255, 255, 255, 0.2) ${first_point * 100}%)`;
  }, 100);
}
// receive tauri emit
document.addEventListener("shaka-ui-loaded", init);
