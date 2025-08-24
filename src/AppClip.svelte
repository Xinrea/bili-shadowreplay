<script lang="ts">
  import { invoke, convertFileSrc, convertCoverSrc } from "./lib/invoker";
  import { onMount } from "svelte";
  import VideoPreview from "./lib/components/VideoPreview.svelte";
  import type { Config, VideoItem } from "./lib/interface";
  import { set_title } from "./lib/invoker";

  let video: VideoItem | null = null;
  let videos: any[] = [];
  let showVideoPreview = false;
  let roomId: number | null = null;

  let config: Config = null;

  invoke("get_config").then((c) => {
    config = c as Config;
  });

  onMount(async () => {
    const videoId = new URLSearchParams(window.location.search).get("id");
    if (videoId) {
      try {
        // 获取视频信息
        const videoData = await invoke("get_video", { id: parseInt(videoId) });
        roomId = (videoData as VideoItem).room_id;
        // update window title to file name
        set_title((videoData as VideoItem).file);
        // 获取房间下的所有视频列表
        if (roomId !== null && roomId !== undefined) {
          const videoList = (await invoke("get_videos", { roomId: roomId })) as VideoItem[];
          videos = await Promise.all(videoList.map(async (v) => {
            return {
              id: v.id,
              value: v.id,
              name: v.file,
              file: await convertFileSrc(v.file),
              cover: v.cover,
            };
          }));
        }

        // find video in videos
        let new_video = videos.find((v) => v.id === parseInt(videoId));

        handleVideoChange(new_video);

        // 显示视频预览
        showVideoPreview = true;
      } catch (error) {
        console.error("Failed to load video:", error);
      }
    }
  });

  async function handleVideoChange(newVideo: VideoItem) {
    if (newVideo) {
      // get cover from video
      const cover = await invoke("get_video_cover", { id: newVideo.id }) as string;
      
      // 对于非空的封面路径，使用convertCoverSrc转换
      if (cover && cover.trim() !== "") {
        newVideo.cover = await convertCoverSrc(cover, newVideo.id);
      } else {
        newVideo.cover = "";
      }
    }
    video = newVideo;
  }

  async function handleVideoListUpdate() {
    if (roomId !== null && roomId !== undefined) {
      const videosData = await invoke("get_videos", { roomId });
      videos = await Promise.all((videosData as VideoItem[]).map(async (v) => {
        return {
          id: v.id,
          value: v.id,
          name: v.file,
          file: await convertFileSrc(v.file),
          cover: v.cover, // 这里保持原样，因为get_videos返回的是VideoNoCover类型，不包含完整封面数据
        };
      }));
    }
  }
</script>

{#if showVideoPreview && video && (roomId !== null && roomId !== undefined)}
  <VideoPreview
    bind:show={showVideoPreview}
    {video}
    {videos}
    {roomId}
    onVideoChange={handleVideoChange}
    onVideoListUpdate={handleVideoListUpdate}
  />
{:else}
  <main
    class="flex items-center justify-center h-screen bg-[#1c1c1e] text-white"
  >
    <div class="text-center">
      <div
        class="animate-spin h-8 w-8 border-2 border-[#0A84FF] border-t-transparent rounded-full mx-auto mb-4"
      ></div>
      <p>加载中...</p>
    </div>
  </main>
{/if}
