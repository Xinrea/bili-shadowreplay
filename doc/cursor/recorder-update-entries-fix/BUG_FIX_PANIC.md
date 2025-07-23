# Panic Bug 修复记录

## 🐛 Bug 描述

在录制非 FMP4 流时，首次录制会发生 panic。原因是在 `entry_store` 初始化之前就尝试访问 `last_sequence`，对 `None` 值调用 `.unwrap()` 导致 panic。

## 📍 问题位置

### 主要问题
**文件**: `src-tauri/src/recorder/bilibili.rs`  
**行数**: 656-665  
**代码**:
```rust
let last_sequence = self
    .entry_store
    .read()
    .await
    .as_ref()
    .unwrap()  // ❌ 这里会 panic
    .last_sequence;
```

### 次要问题
**文件**: `src-tauri/src/recorder/bilibili.rs`  
**行数**: 807  
**代码**:
```rust
if let Some(last_ts) = self.entry_store.read().await.as_ref().unwrap().last_ts() {
    // ❌ 这里也可能 panic
}
```

**文件**: `src-tauri/src/recorder/bilibili.rs`  
**行数**: 868  
**代码**:
```rust
self.entry_store.read().await.as_ref().unwrap().manifest(
    // ❌ 这里也可能 panic
)
```

## 🔄 修复方案

### 1. 安全获取 last_sequence
```rust
// 修复前
let last_sequence = self
    .entry_store
    .read()
    .await
    .as_ref()
    .unwrap()
    .last_sequence;

// 修复后
let last_sequence = self
    .entry_store
    .read()
    .await
    .as_ref()
    .map(|store| store.last_sequence)
    .unwrap_or(0); // For first-time recording, start from 0
```

### 2. 安全检查 last_ts
```rust
// 修复前
if let Some(last_ts) = self.entry_store.read().await.as_ref().unwrap().last_ts() {
    // 检查逻辑
}

// 修复后
if let Some(entry_store) = self.entry_store.read().await.as_ref() {
    if let Some(last_ts) = entry_store.last_ts() {
        // 检查逻辑
    }
}
```

### 3. 安全生成 manifest
```rust
// 修复前
self.entry_store.read().await.as_ref().unwrap().manifest(
    !live_status || range.is_some(),
    true,
    range,
)

// 修复后
if let Some(entry_store) = self.entry_store.read().await.as_ref() {
    entry_store.manifest(
        !live_status || range.is_some(),
        true,
        range,
    )
} else {
    // Return empty manifest if entry_store is not initialized yet
    "#EXTM3U\n#EXT-X-VERSION:3\n".to_string()
}
```

## 🕒 问题时序

### 非 FMP4 流的初始化顺序
1. `check_status()` - 检测到直播，设置 stream URL
2. `update_entries()` - 开始处理播放列表
3. **问题点**: 尝试获取 `last_sequence`，但 `entry_store` 还是 `None`
4. 下载第一个 ts 文件
5. 下载成功后才初始化 `entry_store`

### FMP4 流的初始化顺序（正常）
1. `check_status()` - 检测到直播，设置 stream URL
2. `update_entries()` - 检查需要 header
3. 下载 header 文件
4. 下载成功后立即初始化 `entry_store`
5. 然后处理播放列表 ✅ 此时 `entry_store` 已初始化

## ✅ 验证方法

### 测试场景
1. **正常场景**: FMP4 流应该继续正常工作
2. **修复场景**: 非 FMP4 流首次录制不应该 panic
3. **边界场景**: 空播放列表、网络错误等应该正确处理

### 测试步骤
1. 配置非 FMP4 的直播源
2. 启动录制
3. 确认不会发生 panic
4. 确认录制功能正常

## 🔍 根本原因分析

这个 bug 是在重构过程中引入的，原因是：

1. **设计不一致**: FMP4 和非 FMP4 流的初始化时机不同
2. **隐式假设**: 代码假设 `entry_store` 总是在使用前被初始化
3. **缺少检查**: 没有在访问 `entry_store` 前进行 None 检查

## 📚 经验教训

1. **所有 Option 访问都应该安全**: 避免直接使用 `.unwrap()`
2. **不同代码路径的一致性**: 确保不同条件下的初始化逻辑一致
3. **边界条件测试**: 特别是首次使用的场景
4. **防御性编程**: 即使"理论上不可能"的情况也要处理

## 🔗 相关文档

- [MODIFICATION_SUMMARY.md](./MODIFICATION_SUMMARY.md) - 主要修改说明
- [RECORDER_FLOW_DIAGRAM.md](./RECORDER_FLOW_DIAGRAM.md) - 流程图说明