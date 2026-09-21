//! JCE structs of the Huya danmaku websocket protocol.
//!
//! Field tags mirror Huya's web client `lib.js` (`HUYA` namespace): the
//! `WebSocketCommand` envelope, the register payload (`WSUserInfo`), the wup
//! heartbeat request (`UserHeartBeatReq` inside a v3 `RequestPacket`) and the
//! push chain `WSPushMessage` -> `MessageNotice` (uri 1400).

use super::jce::{JceError, JceReader, JceWriter};

// `HUYA.EWebSocketCommandType`
pub const WS_CMD_REGISTER_REQ: i32 = 1;
pub const WS_CMD_REGISTER_RSP: i32 = 2;
pub const WS_CMD_WUP_REQ: i32 = 3;
pub const WS_CMD_WUP_RSP: i32 = 4;
pub const WS_CMD_S2C_HEARTBEAT_ACK: i32 = 6;
pub const WS_CMD_S2C_MSG_PUSH_REQ: i32 = 7;

// Push uris (`WSPushMessage.iUri`); only the chat notice carries danmaku.
pub const URI_MESSAGE_NOTICE: i64 = 1400;

const HUYA_UA: &str = "webh5&1.0.0&websocket";
const HEARTBEAT_SERVANT: &str = "onlineui";
const HEARTBEAT_FUNC: &str = "OnUserHeartBeat";
const STREAM_LINE_WS: i64 = 1;
const GROUP_TYPE_LIVE: i64 = 3;

/// `HUYA.WebSocketCommand`: `iCmdType` (tag 0) and `vData` (tag 1), written
/// directly at the top level of each websocket frame.
pub struct WebSocketCommand {
    pub cmd_type: i32,
    pub data: Vec<u8>,
}

pub fn encode_websocket_command(cmd_type: i32, data: &[u8]) -> Vec<u8> {
    let mut writer = JceWriter::new();
    writer.write_i64(0, cmd_type as i64);
    writer.write_bytes(1, data);
    writer.into_bytes()
}

pub fn decode_websocket_command(data: &[u8]) -> Result<WebSocketCommand, JceError> {
    let mut reader = JceReader::new(data);
    Ok(WebSocketCommand {
        cmd_type: reader.read_i64(0)?.unwrap_or(0) as i32,
        data: reader.read_bytes(1)?.unwrap_or_default(),
    })
}

/// `HUYA.WSUserInfo` register payload, tagging the connection to the room's
/// live channels and the presenter's chat group (group type 3).
pub fn encode_register(user_id: i64, guid: &str, topsid: i64, subsid: i64) -> Vec<u8> {
    let mut writer = JceWriter::new();
    writer.write_i64(0, user_id);
    writer.write_bool(1, user_id == 0);
    writer.write_str(2, guid);
    writer.write_str(3, "");
    writer.write_i64(4, topsid);
    writer.write_i64(5, subsid);
    writer.write_i64(6, user_id);
    writer.write_i64(7, GROUP_TYPE_LIVE);
    writer.into_bytes()
}

/// `HUYA.WSRegisterRsp.iResCode` (tag 0).
pub fn decode_register_rsp(data: &[u8]) -> Result<Option<i32>, JceError> {
    let mut reader = JceReader::new(data);
    Ok(reader.read_i64(0)?.map(|value| value as i32))
}

/// v3 wup request wrapping `HUYA.UserHeartBeatReq` for
/// `onlineui.OnUserHeartBeat`, including the 4-byte packet length prefix.
pub fn encode_heartbeat_wup(topsid: i64, subsid: i64, pid: i64, request_id: u32) -> Vec<u8> {
    let mut req = JceWriter::new();
    // tag 0: tId (UserId) — only sHuYaUA (tag 3) is set by the web client.
    req.write_struct(0, |writer| {
        writer.write_str(3, HUYA_UA);
    });
    req.write_i64(1, topsid);
    req.write_i64(2, subsid);
    req.write_i64(4, pid);
    req.write_i64(6, STREAM_LINE_WS);
    let req_bytes = req.into_bytes();

    // iVersion 3 puts the request into sBuffer as a map { "tReq": bytes }.
    let mut buffer = JceWriter::new();
    buffer.write_string_bytes_map(0, &[("tReq", &req_bytes)]);
    let buffer_bytes = buffer.into_bytes();

    let mut packet = JceWriter::new();
    packet.write_i64(1, 3);
    packet.write_i64(2, 0);
    packet.write_i64(3, 0);
    packet.write_i64(4, request_id as i64);
    packet.write_str(5, HEARTBEAT_SERVANT);
    packet.write_str(6, HEARTBEAT_FUNC);
    packet.write_bytes(7, &buffer_bytes);
    packet.write_i64(8, 0);
    packet.write_string_bytes_map(9, &[]);
    packet.write_string_bytes_map(10, &[]);
    let packet_bytes = packet.into_bytes();

    let mut frame = Vec::with_capacity(packet_bytes.len() + 4);
    frame.extend_from_slice(&((packet_bytes.len() as u32 + 4).to_be_bytes()));
    frame.extend_from_slice(&packet_bytes);
    frame
}

/// `HUYA.WSPushMessage`: `iUri` (tag 1) and `sMsg` (tag 2).
pub struct PushMessage {
    pub uri: i64,
    pub msg: Vec<u8>,
}

pub fn decode_push_message(data: &[u8]) -> Result<Option<PushMessage>, JceError> {
    let mut reader = JceReader::new(data);
    let Some(uri) = reader.read_i64(1)? else {
        return Ok(None);
    };
    Ok(Some(PushMessage {
        uri,
        msg: reader.read_bytes(2)?.unwrap_or_default(),
    }))
}

/// `HUYA.MessageNotice` (uri 1400): sender info (tag 0), chat content
/// (tag 3) and bullet format (tag 6, carrying the font color).
pub struct MessageNotice {
    pub user_id: i64,
    pub nick_name: String,
    pub content: String,
    pub font_color: i32,
}

pub fn decode_message_notice(data: &[u8]) -> Result<Option<MessageNotice>, JceError> {
    let mut reader = JceReader::new(data);
    let mut notice = MessageNotice {
        user_id: 0,
        nick_name: String::new(),
        content: String::new(),
        font_color: -1,
    };

    // tag 0: tUserInfo (SenderInfo): lUid (0), sNickName (2).
    if reader.enter_struct(0)? {
        notice.user_id = reader.read_i64(0)?.unwrap_or(0);
        notice.nick_name = reader.read_string(2)?.unwrap_or_default();
        reader.leave_struct()?;
    }

    notice.content = reader.read_string(3)?.unwrap_or_default();

    // tag 6: tBulletFormat (BulletFormat): iFontColor (0).
    if reader.enter_struct(6)? {
        notice.font_color = reader.read_i64(0)?.unwrap_or(-1) as i32;
        reader.leave_struct()?;
    }

    Ok(Some(notice))
}
