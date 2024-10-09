<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";

    export let port;
    export let room_id;
    export let ts;
    export let start = 0;
    export let end = 0;
    let show_detail = false;
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
                // location.reload();
            }
        }
        player.addEventListener("ended", async () => {
            location.reload();
        });

        document
            .getElementsByClassName("shaka-overflow-menu-button")[0]
            .remove();
        document.querySelector(
            ".shaka-back-to-overflow-button .material-icons-round",
        ).innerHTML = "arrow_back_ios_new";

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

        function isLive() {
            let total = video.duration;
            if (total == Infinity || total >= 4294967296) {
                return true;
            }
            return false;
        }

        function get_total() {
            let total = video.duration;
            if (total == Infinity || total >= 4294967296) {
                total =
                    (Date.now() - player.getPresentationStartTimeAsDate()) /
                    1000;
            }
            return total;
        }
        // add keydown event listener for '[' and ']' to control range
        document.addEventListener("keydown", async (e) => {
            const target = e.target as HTMLInputElement;
            if (
                (target.tagName.toLowerCase() === "input" &&
                    target.type === "text") ||
                target.tagName.toLowerCase() === "textarea"
            ) {
                return;
            }
            switch (e.key) {
                case "[":
                    if (isLive()) {
                        start = parseFloat(
                            (
                                (player.getPlayheadTimeAsDate() -
                                    player.getPresentationStartTimeAsDate()) /
                                1000
                            ).toFixed(2),
                        );
                    } else {
                        start = parseFloat(video.currentTime.toFixed(2));
                    }
                    if (end < start) {
                        end = get_total();
                    }
                    console.log(start, end);
                    break;
                case "]":
                    if (isLive()) {
                        end = parseFloat(
                            (
                                (player.getPlayheadTimeAsDate() -
                                    player.getPresentationStartTimeAsDate()) /
                                1000
                            ).toFixed(2),
                        );
                    } else {
                        end = parseFloat(video.currentTime.toFixed(2));
                    }
                    if (start > end) {
                        start = 0;
                    }
                    console.log(start, end);
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
                    if (e.altKey) {
                        video.currentTime -= 1;
                    }
                    break;
                case "ArrowRight":
                    if (e.altKey) {
                        video.currentTime += 1;
                    }
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

        setInterval(() => {
            const total = get_total();
            const first_point = start / total;
            const second_point = end / total;
            // set background color for self-defined seekbar between first_point and second_point using linear-gradient
            // example: linear-gradient(to right, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.495542%, rgb(255, 0, 0) 28.63019%, rgba(255, 255, 255, 0.4) 28.63019%, rgba(255, 255, 255, 0.4) 29.285618%, rgba(255, 255, 255, 0.2) 29.285618%)
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
        }, 100);
    }
    // receive tauri emit
    document.addEventListener("shaka-ui-loaded", init);
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
            <p><kbd>Alt</kbd><kbd>←</kbd>前进</p>
            <p><kbd>Alt</kbd><kbd>→</kbd>后退</p>
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
