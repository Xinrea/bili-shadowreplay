# 素材与切片工程

工作流程为 **录播 → 素材 → 切片 → 投稿**：

- **素材**：从录播中截取的一段视频文件，只提供简单预览。生成时不压制弹幕，但会把对应时间段的弹幕截取保存为素材旁的弹幕文件，录播被清理后弹幕仍然可用。
- **切片**：一个简易剪辑工程，由一个或多个素材按顺序组成，在工程中完成裁剪、分割、排序、字幕和弹幕压制，导出后投稿。

## 数据模型

### 素材（`videos` 表）

为了兼容 AI 助手、MCP 与 Webhook，素材沿用 `videos` 表与 `VideoRow`。迁移 18 新增：

| 字段 | 说明 |
| --- | --- |
| `live_id` | 来源录播，导入的视频为空 |
| `source` | 来源信息 JSON：`{"ranges":[{"start","end"}],"local_offset":0}`，时间相对录播播放列表起点 |
| `duration` | 精确时长（秒），旧数据为 0，读取时按需用 ffprobe 补齐 |

素材的旁路文件与视频文件同名：

| 文件 | 说明 |
| --- | --- |
| `<stem>.jpg` | 缩略图 |
| `<stem>.opus` | 波形用音频样本 |
| `<stem>.srt` | AI 识别字幕缓存（素材时间） |
| `<stem>.danmu.json` | 截取的弹幕，`ts` 为相对素材起点的毫秒 |
| `<stem>.peaks.json` | 时间轴波形峰值缓存 |

从录播生成素材时，每个选区生成一个素材（直接复制视频流，不重新编码），并可选同时创建包含这些素材的切片工程。

### 切片（`projects` 表）

| 字段 | 说明 |
| --- | --- |
| `title` / `note` | 标题与备注 |
| `room_id` / `platform` | 所属直播间，用于素材库筛选和投稿配置 |
| `cover` | 投稿封面，相对输出目录（`projects/<id>/cover.jpg`），为空时使用第一个素材的缩略图 |
| `timeline` | 时间轴 JSON，见下文 |
| `profile` | 投稿信息 JSON（B 站 `Profile`） |
| `output` / `output_hash` | 最近一次导出的文件与对应时间轴的哈希；哈希不一致即导出已过期 |
| `status` / `bvid` | 投稿状态 |

工程文件统一放在 `<output>/projects/<id>/`，删除工程时一并删除。

### 时间轴

```json
{
  "version": 1,
  "clips": [{ "id": "c1", "material_id": 12, "start": 3.2, "end": 40.0 }],
  "subtitles": [{ "id": "s1", "clip_id": "c1", "start": 5.0, "end": 7.5, "text": "..." }],
  "subtitle_style": { "fontName": "Arial", "...": "..." },
  "danmu": false
}
```

- `clips` 为主轨道，按顺序首尾相接播放（磁吸主轨），`start` / `end` 是素材内的入点与出点。片段在工程中的起点由前面片段的时长累加得到。
- 字幕**锚定在片段上**，时间是素材时间。片段被移动、裁剪时字幕自动跟随画面；超出片段可见范围的部分被隐藏，重新拉长片段后会恢复。分割片段时，跨越分割点的字幕会复制到两侧。
- `danmu` 为 `true` 时，导出会把各片段可见范围内的素材弹幕映射到工程时间后压制。

## 预览与导出

- **预览**：两个 `<video>` 交替播放，在切点处无缝切换；对外暴露一个以工程时间为准的虚拟媒体时钟，libass 字幕预览直接绑定该时钟。预览依赖普通的 HTTP 静态文件，桌面端与 Docker 网页端一致。
- **导出**：后端把时间轴编译为一条 FFmpeg 命令：每个片段用 `-ss/-t` 作为独立输入，统一缩放到 1920×1080、统一帧率与音频格式后 `concat`，再依次叠加字幕（前端生成的 ASS，与预览完全一致）和弹幕（ASS + 表情图片叠加），经过任务队列执行，支持进度与取消。
- 只有一个完整素材、没有字幕和弹幕的工程无需导出，投稿时直接使用素材文件。

## 接口

| 命令 | 说明 |
| --- | --- |
| `generate_materials` | 从录播选区生成素材（每个选区一个），可选同时创建工程 |
| `get_material_usage` / `get_material_peaks` / `get_material_danmu` | 素材被哪些工程使用 / 波形峰值 / 截取的弹幕 |
| `get_projects` / `get_project` / `create_project` / `delete_project` | 工程增删查 |
| `update_project_info` / `update_project_timeline` / `update_project_profile` / `update_project_cover` | 工程修改 |
| `export_project` | 导出工程，返回任务或说明无需导出 |
| `upload_project` | 投稿工程的导出文件 |
| `open_project` / `open_clip` | 打开工程编辑窗口 / 素材预览窗口 |

`clip_range` 仍然可用（AI 助手与 MCP 使用），它把所有选区合并为一个素材，同样不再压制弹幕。

## 旧数据迁移

迁移 18 为已有的每条 `videos` 记录标记 `needs_project`，启动时为它们各创建一个只含该素材的工程：标题、备注、投稿状态与 BV 号迁到工程，素材的 `.srt` 字幕导入为工程字幕。
