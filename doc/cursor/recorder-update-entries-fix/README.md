# Recorder Update Entries 修复任务

> **任务目标**：修改 recorder 的 `update_entries` 函数，使其仅在成功下载 ts 文件后才创建记录和目录，避免产生空记录和空目录。

## 📋 任务概述

### 问题描述
目前 recorder 的 `update_entries` 遇到错误时会重新获取 stream 地址，并使用时间戳作为 `live_id` 创建新的记录和目录。这导致即使后续下载 ts 文件失败，也会产生空的记录和目录。

### 解决方案
实现"仅当成功下载 ts 文件后才创建记录和目录"的逻辑，同时在下载失败时清理空目录。

## 📁 文档结构

```
doc/cursor/recorder-update-entries-fix/
├── README.md                    # 本文件，任务说明
├── MODIFICATION_SUMMARY.md     # 详细修改总结
└── RECORDER_FLOW_DIAGRAM.md    # 执行流程图
```

## 📄 文档说明

### [MODIFICATION_SUMMARY.md](./MODIFICATION_SUMMARY.md)
**详细修改总结**
- 问题分析和解决方案
- 具体代码修改内容
- 目录清理策略
- 测试建议和注意事项

### [RECORDER_FLOW_DIAGRAM.md](./RECORDER_FLOW_DIAGRAM.md)
**执行流程图**
- Bilibili 和 Douyin Recorder 的完整流程
- 修改前后的行为对比
- 错误处理流程
- 状态管理对比

## 🔧 主要修改

### 影响的文件
- `src-tauri/src/recorder/bilibili.rs`
- `src-tauri/src/recorder/douyin.rs`

### 核心改动
1. **延迟创建记录**：只有在成功下载内容后才创建数据库记录
2. **目录清理机制**：下载失败时自动删除空目录
3. **优化初始化逻辑**：将存储初始化延迟到实际需要时

## 🎯 改进效果

- ✅ **避免空记录**：数据库中不会有无内容的记录
- ✅ **避免空目录**：文件系统中不会留下空的录制目录
- ✅ **更好的错误处理**：所有失败场景都有对应的清理逻辑
- ✅ **资源优化**：避免创建不必要的对象和任务

## 📝 开发记录

- **开发时间**：2024年
- **开发工具**：Cursor AI
- **开发方式**：背景代理模式
- **文档类型**：技术分析和修改说明

## 🔗 相关资源

- **源代码位置**：`src-tauri/src/recorder/`
- **测试建议**：参见 [MODIFICATION_SUMMARY.md](./MODIFICATION_SUMMARY.md#测试建议)
- **流程图**：参见 [RECORDER_FLOW_DIAGRAM.md](./RECORDER_FLOW_DIAGRAM.md)

---

> 💡 **注意**：本任务的修改涉及录制的核心逻辑，建议在部署前进行充分测试。