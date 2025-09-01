# Webhook

> [!NOTE]
> 你可以使用 <https://webhook.site> 来测试 Webhook 功能。

## 设置 Webhook

1. 打开 BSR 设置页面
2. 找到 Webhook 设置
3. 输入 Webhook URL
4. 点击保存

## Basic Objects

### UserObject

```json
{
  "user_id": "123456",
  "user_name": "测试主播",
  "user_avatar": "https://example.com/avatar.jpg"
}
```

### RoomObject

```json
{
  "room_id": 123456,
  "platform": "bilibili",
  "room_title": "测试直播间",
  "room_cover": "https://example.com/cover.jpg",
  "room_owner": UserObject
}
```

### LiveObject

```json
{
  "live_id": "123456",
  "room": RoomObject,
  "start_time": 1719849600,
  "end_time": 1719849600
}
```

### ClipObject

```json
{
  "clip_id": 123456,
  "live": LiveObject,
  "range": {
    "start": 0,
    "end": 0
  },
  "note": "测试切片",
  "cover": "https://example.com/cover.jpg",
  "file": "test.mp4",
  "size": 123456,
  "length": 33,
  "with_danmaku": false,
  "with_subtitle": false
}
```

### EventObject

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "room.added",
  "payload": Object,
  "timestamp": 1719849600
}
```

## Webhook Events

### 直播间相关

#### 添加直播间

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "room.added",
  "payload": {
    "room_id": 123456,
    "platform": "bilibili",
    "room_title": "测试直播间",
    "room_cover": "https://example.com/cover.jpg",
    "room_owner": {
      "user_id": "123456",
      "user_name": "测试主播",
      "user_avatar": "https://example.com/avatar.jpg"
    }
  },
  "timestamp": 1719849600
}
```

#### 移除直播间

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "room.removed",
  "payload": {
    "room_id": 123456,
    "platform": "bilibili",
    "room_title": "测试直播间",
    "room_cover": "https://example.com/cover.jpg",
    "room_owner": {
      "user_id": "123456",
      "user_name": "测试主播",
      "user_avatar": "https://example.com/avatar.jpg"
    }
  },
  "timestamp": 1719849600
}
```

### 直播相关

> [!NOTE]
> 直播开始和结束，不意味着录制的开始和结束。

#### 直播开始

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "live.started",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "start_time": 1719849600,
    "end_time": null
  },
  "timestamp": 1719849600
}
```

#### 直播结束

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "live.ended",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "start_time": 1719849600,
    "end_time": 1719849600
  },
  "timestamp": 1719849600
}
```

### 录播相关

#### 开始录制

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "stream.started",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "start_time": 1719849600,
    "end_time": null
  },
  "timestamp": 1719849600
}
```

#### 结束录制

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "stream.ended",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "start_time": 1719849600,
    "end_time": 1719849600
  },
  "timestamp": 1719849600
}
```

#### 删除录制

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "stream.deleted",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    }
    "start_time": 1719849600,
    "end_time": 1719849600
  },
  "timestamp": 1719849600
}
```

### 切片相关

#### 切片生成

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "clip.generated",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "clip_id": 123456,
    "clip_title": "测试切片",
    "clip_file": "test.mp4",
    "clip_length": 33
  },
  "timestamp": 1719849600
}
```

#### 切片删除

```json
{
  "id": "23dd6c5d-c33b-4023-8b00-ec60e8abee3c",
  "event": "clip.deleted",
  "payload": {
    "live_id": "123456",
    "room": {
      "room_id": 123456,
      "platform": "bilibili",
      "room_title": "测试直播间",
      "room_cover": "https://example.com/cover.jpg",
      "room_owner": {
        "user_id": "123456",
        "user_name": "测试主播",
        "user_avatar": "https://example.com/avatar.jpg"
      }
    },
    "clip_id": 123456,
    "clip_title": "测试切片",
    "clip_file": "test.mp4",
    "clip_length": 33
  },
  "timestamp": 1719849600
}
```
