# MCP Server

BSR 提供 MCP Server，让支持 MCP 的外部 Agent 完成从录播到投稿草稿的流程：查询直播间和录播、根据字幕和弹幕挑选时间点、切出素材、把素材编排成切片，并为素材或切片创建投稿草稿。投稿草稿由用户在 BSR 的投稿页检查后自行发布，MCP 不会发布视频。服务采用 [MCP Streamable HTTP 传输](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#streamable-http)，支持协议版本 `2025-11-25`、`2025-06-18` 和 `2025-03-26`。

## 启用服务

打开 BSR 的设置页面，在 **MCP Server** 中打开 **启用 MCP Server**。开关立即启动或停止服务，配置会自动保存，重启 BSR 后保持原来的设置。页面会显示实际运行状态和连接地址。

默认端口为 `18763`。如需更改，在 **MCP 端口** 中输入 `1024–65535` 之间的整数，然后点击 **保存端口**。服务运行时会立即切换到新端口；服务关闭时也会保存配置，下一次启用时使用新端口。未点击保存的输入不会影响服务开关或页面中的连接地址。

端口被占用时，BSR 会显示明确的错误，不会自动选择其他端口。运行中的服务更换端口失败时，原端口和原服务保持可用。选择空闲端口后重新保存即可；如果初次启用失败，可以先保存空闲端口，再启用服务。

MCP Server 默认关闭。桌面模式和 headless 模式均可使用，也可通过连接 headless API 的网页设置页面控制开关。在 headless 模式下，可以编辑 `config.toml` 的根配置项并重新启动 BSR：

```toml
mcp_enabled = true
mcp_port = 18763
```

这两个配置项应放在 `[llm]`、`[auto_generate]` 等表之前。可以将 `mcp_port` 改为其他有效端口。

## 连接外部 Agent

在外部 Agent 的 MCP 配置中新增服务，选择 **HTTP / Streamable HTTP** 传输，并填写设置页面显示的连接地址。默认地址为：

```text
http://127.0.0.1:18763/mcp
```

设置页面提供地址和配置示例的复制按钮。以下示例适用于使用 `mcpServers` 格式且支持 `type: "http"` 的客户端；字段名称和传输类型的写法因客户端而异，请以客户端说明为准，连接时使用完整的 HTTP URL：

```json
{
  "mcpServers": {
    "bsr": {
      "type": "http",
      "url": "http://127.0.0.1:18763/mcp"
    }
  }
}
```

如果使用自定义端口，请替换示例 URL 中的 `18763`。保存新的端口后，必须同步更新外部 Agent 的 MCP URL，并重新连接。

BSR 必须保持运行。客户端连接后会自动发现当前可用工具，可以先让 Agent 列出直播间或查询录播，确认连接正常。

## 可用操作

桌面模式和 headless 模式提供以下工具。只读 headless 模式会隐藏并拒绝执行其中的修改工具。

| 范围 | 操作 | 工具 |
| --- | --- | --- |
| 账号 | 查询已配置账号、移除账号 | `get_accounts`、`remove_account` |
| 直播间 | 查询直播间和录制状态、添加或移除直播间 | `get_recorder_list`、`get_recorder_info`、`add_recorder`、`remove_recorder` |
| 录播 | 查询列表和详情、查询近期录制、删除录播 | `get_archives`、`get_archive`、`get_recent_record`、`get_recent_record_all`、`delete_archive`、`delete_archives` |
| 弹幕 | 读取弹幕（可按时间段）、分析弹幕高峰、搜索关键词 | `get_danmu_record`、`analyze_danmu_highlights`、`search_danmu_keywords` |
| 素材 | 查询列表、详情和封面，从录播生成素材，删除素材 | `get_videos`、`get_all_videos`、`get_video`、`get_video_cover`、`generate_materials`、`clip_range`、`delete_video` |
| 字幕 | 读取或生成录播及素材字幕、将字幕压制到素材 | `get_archive_subtitle`、`get_video_subtitle`、`generate_archive_subtitle`、`generate_video_subtitle`、`encode_video_subtitle` |
| 切片 | 查询、新建和删除切片，设置主轨片段，修改标题备注，设置弹幕压制和画质码率 | `get_projects`、`get_project`、`create_project`、`set_project_clips`、`update_project_info`、`update_project_output_settings`、`delete_project` |
| 投稿草稿 | 查询分区，查询投稿记录，创建和修改草稿，设置是否压制弹幕，删除记录 | `get_video_typelist`、`get_uploads`、`get_upload`、`create_upload_draft`、`update_upload_draft`、`update_upload_danmu`、`delete_upload` |
| 后台任务 | 查询任务列表或单个任务、取消任务、删除任务记录 | `get_background_tasks`、`get_task`、`cancel_task`、`delete_background_task` |

### 时间基准

`get_danmu_record`、`analyze_danmu_highlights` 和 `search_danmu_keywords` 返回的时间都是**从录播开始算起的秒数**，与 `generate_materials`、`clip_range` 的 `ranges` 使用同一基准，可以直接用作切片区间。录播开始之前收到的弹幕不会返回。`get_danmu_record` 可以用 `start`、`end`（同样以秒计）只读取一段时间内的弹幕，避免一次返回整场直播的弹幕。

### 生成素材

`generate_materials` 与素材生成对话框一致：默认每个选区生成一个素材；`merge` 为 `true` 时按顺序合并为一个素材；`create_project` 为 `true` 时同时创建一个依次播放这些素材的切片。`clip_range` 始终把所有选区合并为一个素材。两者的 `clip_range_params` 只要求 `room_id`、`live_id`、`platform`、`title` 和 `ranges`，其余字段可省略：`note`、`cover` 默认为空（自动截取封面），`danmu`、`fix_encoding` 默认为 `false`，`local_offset` 默认为 `0`。

### 后台任务

`generate_materials`、`clip_range`、`generate_video_subtitle` 和 `generate_archive_subtitle` 在后台执行，调用后立即返回 `task_id`。用 `get_task` 查询，直到 `status` 为 `success`、`failed` 或 `cancelled`；成功后 `metadata.result` 中是产出的 ID，例如素材的 `video_id`，或 `material_ids` 与 `project_id`。字幕生成完成后用 `get_video_subtitle` 或 `get_archive_subtitle` 读取。开始执行前就能发现的错误（例如素材或录播不存在）会直接作为调用错误返回。在任务队列中运行的任务可以用 `cancel_task` 取消；字幕识别不在任务队列中，无法取消。

`encode_video_subtitle` 同样立即返回，结果中的 `task_id` 可用于 `get_task`。返回排队任务不代表视频已压制完成。

### 切片

`create_project` 为每个素材创建一个完整长度的片段，片段会自动带上素材已识别的字幕。`get_project` 返回解析后的时间轴：`clips` 依次首尾相接播放，`start`、`end` 是片段在素材内的入点和出点（秒）。

`set_project_clips` 按给定顺序替换整条主轨。传入已有片段的 `clip_id`（且素材相同）时保留该片段，锚定在它上面的字幕、文字、贴纸、画中画和音频随之保留；未传入的片段及锚定在其上的内容会被移除。新片段会自动带上素材已识别的字幕。切片正在编辑窗口中打开时不要调用，编辑窗口保存时会覆盖这次修改。

### 投稿草稿

`create_upload_draft` 为素材或切片创建投稿草稿，可填写标题、简介、标签、分区（`tid`，用 `get_video_typelist` 查询）、动态和投稿账号（`uid`，用 `get_accounts` 查询 B 站账号）。未填写的字段使用投稿页的默认值，封面默认使用来源的封面。`update_upload_draft` 只修改传入的字段。

MCP 不提供发布工具。草稿出现在 BSR 的投稿页，由用户检查投稿信息、封面后点击发布；切片中的字幕、文字和贴纸也在发布时生成。投稿进行中或已发布的记录无法再修改。

`get_video_cover` 会读取输出目录内的封面文件，或解码数据库中遗留的 Base64 图片 data URL，返回图片内容块，并附带视频 ID 和封面文件名（内嵌封面为 `null`）。支持 JPEG、PNG、GIF 和 WebP，MIME 类型按图片内容判断；没有封面、文件无法读取或格式不支持时返回工具错误。

`extract_video_frames` 在桌面模式和 headless 模式均可使用，返回 JPEG 图片内容块和对应时间戳。运行环境需要提供 FFmpeg 和 ffprobe，并能读取素材视频文件；只读 headless 模式也可使用此工具。

以下工具依赖现有桌面视频处理模块，仅桌面模式提供；headless 模式不会显示这些工具。

| 操作 | 工具 |
| --- | --- |
| 读取视频或录播的媒体元数据 | `get_video_metadata`、`get_archive_metadata` |
| 合并素材 | `merge_videos` |
| 提取视频音频 | `extract_video_audio` |

具体工具、参数和说明由 MCP 的工具列表提供。MCP Server 不提供任意 FFmpeg 命令执行、打开桌面窗口或发布投稿的工具。

启用服务后，同机外部 Agent 可以访问 BSR 中的内容，并执行工具列表中的修改和删除操作。这些调用由外部 MCP 客户端负责向用户确认，不会进入 BSR 内置 AI 助手的审批流程。只读 headless 模式仅提供查询工具，并在服务端拒绝修改操作。

## 本机地址与 Docker

MCP Server 使用单独配置的端口（默认 `18763`），仅监听 BSR 所在机器或容器的 `127.0.0.1`。它不随 headless HTTP API 的监听地址或端口改变。

桌面 BSR 和 Agent 在同一台机器上时可直接使用上面的地址。如果网页设置连接的是另一台机器上的 BSR，所显示的 `127.0.0.1` 指的是 BSR 所在机器，无法直接从网页所在机器访问；需让 Agent 在 BSR 所在机器运行，或配置适当的本机端口转发。

在 Docker 中，`127.0.0.1` 指容器内部。Agent 需要位于同一容器或共享网络命名空间，也可以使用能够连接到容器内部回环地址的转发方式。**仅映射容器的 MCP 端口到宿主机不足以访问服务**，因为服务没有监听容器的外部网络接口。

## 连接排查

- 设置显示“已关闭”：先启用 MCP Server。
- 设置显示端口占用或启动错误：检查设置中的 MCP 端口是否被其他进程占用，选择空闲端口并保存，或释放原端口后再次启用。BSR 不会自动更换端口。
- 更改端口后客户端无法连接：将外部 Agent 的 MCP URL 更新为设置中显示的新地址，并重新连接。
- 客户端连接失败：确认使用 HTTP / Streamable HTTP 传输、完整的 `/mcp` 路径，并确认 Agent 与 BSR 的 `127.0.0.1` 位于同一台机器或网络命名空间。
- 只能看到查询工具：检查 BSR 是否以只读 headless 模式启动。
- 找不到媒体元数据、音频提取或合并工具：这些工具仅桌面模式提供。
- 视频帧提取失败：确认 FFmpeg 和 ffprobe 可用，且配置的输出目录中存在对应视频文件。
- 网页设置中无法复制：浏览器可能限制了剪贴板访问，可直接选择地址或配置示例手动复制。
