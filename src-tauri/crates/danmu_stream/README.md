## Event format

The stream providers expose every decoded websocket event as a
`DanmuMessageType::Event`. Recorders write these events as one JSON object per
line to `events.jsonl`:

```json
{
  "ts": 1710000000123,
  "platform": "bilibili",
  "room_id": "12345",
  "type": "gift",
  "data": {
    "user_id": 42,
    "user_name": "viewer",
    "gift_name": "花束",
    "count": 1,
    "price": 100
  },
  "raw": {
    "cmd": "SEND_GIFT",
    "data": {}
  }
}
```

`ts` is Unix time in milliseconds and `type` is the normalized event
category. Current categories are `danmu`, `gift`, `super_chat`, `enter`,
`like`, `online`, `room_update`, and `unknown`. `data` has the common fields
for that category; `raw` retains the original JSON provider payload so that
new provider fields remain available without changing the storage format.
