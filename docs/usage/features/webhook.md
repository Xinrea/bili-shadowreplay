# Webhook

> [!NOTE]
> 你可以使用 <https://webhook.site> 来测试 Webhook 功能。

## 设置 Webhook

打开 BSR 设置页面，在基础设置中设置 Webhook 地址。

## Webhook Events

### 直播间相关

#### 添加直播间

```json
{
  "id": "a96a5e9f-9857-4c13-b889-91da2ace208a",
  "event": "recorder.added",
  "payload": {
    "room_id": 26966466,
    "created_at": "2025-09-07T03:33:14.258796+00:00",
    "platform": "bilibili",
    "auto_start": true,
    "extra": ""
  },
  "timestamp": 1757215994
}
```

#### 移除直播间

```json
{
  "id": "e33623d4-e040-4390-88f5-d351ceeeace7",
  "event": "recorder.removed",
  "payload": {
    "room_id": 27183290,
    "created_at": "2025-08-30T10:54:18.569198+00:00",
    "platform": "bilibili",
    "auto_start": true,
    "extra": ""
  },
  "timestamp": 1757217015
}
```

### 直播相关

> [!NOTE]
> 直播开始和结束，不意味着录制的开始和结束。

#### 直播开始

```json
{
  "id": "f12f3424-f7d8-4b2f-a8b7-55477411482e",
  "event": "live.started",
  "payload": {
    "room_id": 843610,
    "room_info": {
      "room_id": 843610,
      "room_title": "登顶！",
      "room_cover": "https://i0.hdslb.com/bfs/live/new_room_cover/73aea43f4b4624c314d62fea4b424822fb506dfb.jpg"
    },
    "user_info": {
      "user_id": "475210",
      "user_name": "Xinrea",
      "user_avatar": "https://i1.hdslb.com/bfs/face/91beb3bf444b295fe12bae1f3dc6d9fc4fe4c224.jpg"
    },
    "total_length": 0,
    "current_live_id": "",
    "live_status": false,
    "is_recording": false,
    "auto_start": true,
    "platform": "bilibili"
  },
  "timestamp": 1757217190
}
```

#### 直播结束

```json
{
  "id": "e8b0756a-02f9-4655-b5ae-a170bf9547bd",
  "event": "live.ended",
  "payload": {
    "room_id": 843610,
    "room_info": {
      "room_id": 843610,
      "room_title": "登顶！",
      "room_cover": "https://i0.hdslb.com/bfs/live/new_room_cover/73aea43f4b4624c314d62fea4b424822fb506dfb.jpg"
    },
    "user_info": {
      "user_id": "475210",
      "user_name": "Xinrea",
      "user_avatar": "https://i1.hdslb.com/bfs/face/91beb3bf444b295fe12bae1f3dc6d9fc4fe4c224.jpg"
    },
    "total_length": 0,
    "current_live_id": "",
    "live_status": true,
    "is_recording": false,
    "auto_start": true,
    "platform": "bilibili"
  },
  "timestamp": 1757217365
}
```

### 录播相关

#### 开始录制

```json
{
  "id": "5ec1ea10-2b31-48fd-8deb-f2d7d2ea5985",
  "event": "record.started",
  "payload": {
    "room_id": 26966466,
    "room_info": {
      "room_id": 26966466,
      "room_title": "早安獭獭栞！下播前抽fufu",
      "room_cover": "https://i0.hdslb.com/bfs/live/user_cover/b810c36855168034557e905e5916b1dba1761fa4.jpg"
    },
    "user_info": {
      "user_id": "1609526545",
      "user_name": "栞栞Shiori",
      "user_avatar": "https://i1.hdslb.com/bfs/face/47e8dbabb895de44ec6cace085d4dc1d40307277.jpg"
    },
    "total_length": 0,
    "current_live_id": "1757216045412",
    "live_status": true,
    "is_recording": false,
    "auto_start": true,
    "platform": "bilibili"
  },
  "timestamp": 1757216045
}
```

#### 结束录制

```json
{
  "id": "56fd03e5-3965-4c2e-a6a9-bb6932347eb3",
  "event": "record.ended",
  "payload": {
    "room_id": 26966466,
    "room_info": {
      "room_id": 26966466,
      "room_title": "早安獭獭栞！下播前抽fufu",
      "room_cover": "https://i0.hdslb.com/bfs/live/user_cover/b810c36855168034557e905e5916b1dba1761fa4.jpg"
    },
    "user_info": {
      "user_id": "1609526545",
      "user_name": "栞栞Shiori",
      "user_avatar": "https://i1.hdslb.com/bfs/face/47e8dbabb895de44ec6cace085d4dc1d40307277.jpg"
    },
    "total_length": 52.96700000000001,
    "current_live_id": "1757215994597",
    "live_status": true,
    "is_recording": true,
    "auto_start": true,
    "platform": "bilibili"
  },
  "timestamp": 1757216040
}
```

#### 删除录播

```json
{
  "id": "c32bc811-ab4b-49fd-84c7-897727905d16",
  "event": "archive.deleted",
  "payload": {
    "platform": "bilibili",
    "live_id": "1756607084705",
    "room_id": 1967212929,
    "title": "灶台O.o",
    "length": 9,
    "size": 1927112,
    "created_at": "2025-08-31T02:24:44.728616+00:00",
    "cover": "bilibili/1967212929/1756607084705/cover.jpg"
  },
  "timestamp": 1757176219
}
```

### 切片相关

#### 切片生成

```json
{
  "id": "f542e0e1-688b-4f1a-8ce1-e5e51530cf5d",
  "event": "clip.generated",
  "payload": {
    "id": 316,
    "room_id": 27183290,
    "cover": "[27183290][1757172501727][一起看凡人修仙传][2025-09-07_00-16-11].jpg",
    "file": "[27183290][1757172501727][一起看凡人修仙传][2025-09-07_00-16-11].mp4",
    "note": "",
    "length": 121,
    "size": 53049119,
    "status": 0,
    "bvid": "",
    "title": "",
    "desc": "",
    "tags": "",
    "area": 0,
    "created_at": "2025-09-07T00:16:11.747461+08:00",
    "platform": "bilibili"
  },
  "timestamp": 1757175371
}
```

#### 切片删除

```json
{
  "id": "5c7ca728-753d-4a7d-a0b4-02c997ad2f92",
  "event": "clip.deleted",
  "payload": {
    "id": 313,
    "room_id": 27183290,
    "cover": "[27183290][1756903953470][不出非洲之心不下播][2025-09-03_21-10-54].jpg",
    "file": "[27183290][1756903953470][不出非洲之心不下播][2025-09-03_21-10-54].mp4",
    "note": "",
    "length": 32,
    "size": 18530098,
    "status": 0,
    "bvid": "",
    "title": "",
    "desc": "",
    "tags": "",
    "area": 0,
    "created_at": "2025-09-03T21:10:54.943682+08:00",
    "platform": "bilibili"
  },
  "timestamp": 1757147617
}
```
