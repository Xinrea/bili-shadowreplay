# 状态管理 (Stores)

## 概述

BiliBili ShadowReplay 使用 Svelte 的响应式存储 (Stores) 来管理全局状态。所有 stores 位于 `src/lib/stores/` 目录。

## Svelte Stores 基础

Svelte 提供三种基本的 store 类型：

- **writable**: 可读写的存储
- **readable**: 只读存储
- **derived**: 派生存储，基于其他 stores 计算得出

在组件中使用 `$` 前缀可以自动订阅和解除订阅 store。

```typescript
import { writable } from 'svelte/store';

// 创建 store
export const count = writable(0);

// 在组件中使用
<script>
  import { count } from './stores';
  // $count 会自动订阅并在值变化时更新
</script>

<p>Count: {$count}</p>
```

## 主要 Stores

### 应用状态 Stores

用于管理应用级别的全局状态，如用户配置、主题设置等。

### 录制状态 Stores

管理录制任务的状态，包括：
- 当前录制任务列表
- 录制进度
- 录制状态 (录制中/已停止/错误)

### 直播间 Stores

管理直播间信息：
- 直播间列表
- 直播间状态 (在线/离线)
- 直播间配置

### 任务 Stores

管理后台任务状态：
- 任务队列
- 任务进度
- 任务结果

## 使用模式

### 订阅 Store

```typescript
import { myStore } from '$lib/stores';

// 方式 1: 在组件中使用 $ 前缀 (推荐)
<script>
  import { myStore } from '$lib/stores';
</script>
<div>{$myStore}</div>

// 方式 2: 手动订阅
const unsubscribe = myStore.subscribe(value => {
  console.log(value);
});
// 记得在组件销毁时取消订阅
onDestroy(unsubscribe);
```

### 更新 Store

```typescript
import { writable } from 'svelte/store';

const count = writable(0);

// 方式 1: set 方法
count.set(1);

// 方式 2: update 方法
count.update(n => n + 1);

// 方式 3: 在组件中直接赋值 (使用 $ 前缀)
$count = 5;
$count += 1;
```

### 派生 Store

```typescript
import { derived } from 'svelte/store';
import { count } from './stores';

// 创建派生 store
export const doubled = derived(count, $count => $count * 2);

// 从多个 stores 派生
export const sum = derived(
  [store1, store2],
  ([$store1, $store2]) => $store1 + $store2
);
```

## 最佳实践

1. **保持 Store 简单**: 每个 store 应该只管理一个关注点
2. **使用 TypeScript**: 为 store 定义明确的类型
3. **避免嵌套订阅**: 使用 derived stores 而不是在订阅中再订阅
4. **命名约定**: 使用描述性的名称，如 `recordingStatus` 而不是 `status`
5. **初始化**: 为 stores 提供合理的初始值

## 示例

### 创建一个录制状态 Store

```typescript
import { writable, derived } from 'svelte/store';

interface Recording {
  id: string;
  roomId: string;
  status: 'recording' | 'stopped' | 'error';
  progress: number;
}

// 创建 writable store
export const recordings = writable<Recording[]>([]);

// 创建派生 store - 正在录制的数量
export const activeRecordingsCount = derived(
  recordings,
  $recordings => $recordings.filter(r => r.status === 'recording').length
);

// 辅助函数
export function addRecording(recording: Recording) {
  recordings.update(list => [...list, recording]);
}

export function updateRecordingProgress(id: string, progress: number) {
  recordings.update(list =>
    list.map(r => r.id === id ? { ...r, progress } : r)
  );
}

export function removeRecording(id: string) {
  recordings.update(list => list.filter(r => r.id !== id));
}
```

### 在组件中使用

```svelte
<script lang="ts">
  import { recordings, activeRecordingsCount } from '$lib/stores/recordings';
  import { onMount } from 'svelte';

  onMount(() => {
    // 组件挂载时的初始化逻辑
  });
</script>

<div>
  <h2>活跃录制: {$activeRecordingsCount}</h2>

  {#each $recordings as recording}
    <div class="recording-item">
      <span>{recording.roomId}</span>
      <span>{recording.status}</span>
      <progress value={recording.progress} max="100"></progress>
    </div>
  {/each}
</div>
```

## 与 Tauri 集成

Stores 通常与 Tauri 命令配合使用，从后端获取数据并更新前端状态：

```typescript
import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

export const rooms = writable([]);

// 从后端加载数据
export async function loadRooms() {
  try {
    const data = await invoke('get_rooms');
    rooms.set(data);
  } catch (error) {
    console.error('Failed to load rooms:', error);
  }
}

// 监听后端事件
import { listen } from '@tauri-apps/api/event';

listen('room-status-changed', (event) => {
  rooms.update(list => {
    // 更新对应房间的状态
    return list.map(room =>
      room.id === event.payload.id
        ? { ...room, status: event.payload.status }
        : room
    );
  });
});
```

## 调试

使用 Svelte DevTools 可以查看和调试 stores 的状态变化。

在开发模式下，可以在浏览器控制台中访问 stores：

```typescript
// 在开发环境中暴露 stores 用于调试
if (import.meta.env.DEV) {
  window.__stores__ = {
    recordings,
    rooms,
    // ... 其他 stores
  };
}
```