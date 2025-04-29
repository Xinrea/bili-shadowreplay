use crate::{
    config::Config,
    database::{
        account::AccountRow, message::MessageRow, record::RecordRow, recorder::RecorderRow,
        video::VideoRow,
    },
    handlers::{
        account::{
            add_account, get_account_count, get_accounts, get_qr, get_qr_status, remove_account,
        },
        config::{
            get_config, set_cache_path, set_output_path, update_auto_generate,
            update_clip_name_format, update_notify, update_subtitle_setting, update_whisper_model,
            update_whisper_prompt,
        },
        message::{delete_message, get_messages, read_message},
        recorder::{
            add_recorder, delete_archive, fetch_hls, force_start, force_stop, get_archive,
            get_archives, get_danmu_record, get_recent_record, get_recorder_list, get_room_info,
            get_today_record_count, get_total_length, remove_recorder, send_danmaku,
            set_auto_start,
        },
        utils::{get_disk_info, DiskInfo},
        video::{
            cancel, clip_range, delete_video, encode_video_subtitle, generate_video_subtitle,
            get_video, get_video_subtitle, get_video_typelist, get_videos, update_video_cover,
            update_video_subtitle, upload_procedure,
        },
        AccountInfo,
    },
    progress_manager::Event,
    recorder::{
        bilibili::{
            client::{QrInfo, QrStatus},
            profile::Profile,
            response::Typelist,
        },
        danmu::DanmuEntry,
        RecorderInfo,
    },
    recorder_manager::{ClipRangeParams, RecorderList},
    state::State,
};
use axum::response::sse;
use axum::{
    extract::{Json, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Sse},
    routing::{get, post},
    Router,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncSeekExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse<T> {
    code: u32,
    message: String,
    data: Option<T>,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
            data: Some(data),
        }
    }

    fn error(message: String) -> Self {
        Self {
            code: 1,
            message,
            data: None,
        }
    }
}

async fn handler_get_accounts(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<AccountInfo>>, String> {
    let mut accounts = get_accounts(state.0).await?;
    for account in accounts.accounts.iter_mut() {
        account.cookies = "".to_string();
    }
    Ok(Json(ApiResponse::success(accounts)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddAccountRequest {
    platform: String,
    cookies: String,
}

async fn handler_add_account(
    state: axum::extract::State<State>,
    Json(param): Json<AddAccountRequest>,
) -> Result<Json<ApiResponse<AccountRow>>, String> {
    let mut account = add_account(state.0, param.platform, &param.cookies).await?;
    account.cookies = "".to_string();
    Ok(Json(ApiResponse::success(account)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveAccountRequest {
    platform: String,
    uid: u64,
}

async fn handler_remove_account(
    state: axum::extract::State<State>,
    Json(account): Json<RemoveAccountRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    remove_account(state.0, account.platform, account.uid).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_account_count(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<u64>>, String> {
    let count = get_account_count(state.0).await?;
    Ok(Json(ApiResponse::success(count)))
}

async fn handler_get_qr(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<QrInfo>>, String> {
    let qr = get_qr(state.0).await.expect("Failed to get QR code");
    Ok(Json(ApiResponse::success(qr)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetQrStatusRequest {
    qrcode_key: String,
}

async fn handler_get_qr_status(
    state: axum::extract::State<State>,
    Json(qr_info): Json<GetQrStatusRequest>,
) -> Result<Json<ApiResponse<QrStatus>>, String> {
    let qr_status = get_qr_status(state.0, &qr_info.qrcode_key)
        .await
        .expect("Failed to get QR status");
    Ok(Json(ApiResponse::success(qr_status)))
}

async fn handler_get_config(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Config>>, String> {
    let config = get_config(state.0).await.expect("Failed to get config");
    Ok(Json(ApiResponse::success(config)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetCachePathRequest {
    cache_path: String,
}

async fn handler_set_cache_path(
    state: axum::extract::State<State>,
    Json(cache_path): Json<SetCachePathRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    set_cache_path(state.0, cache_path.cache_path)
        .await
        .expect("Failed to set cache path");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetOutputPathRequest {
    output_path: String,
}

async fn handler_set_output_path(
    state: axum::extract::State<State>,
    Json(output_path): Json<SetOutputPathRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    set_output_path(state.0, output_path.output_path)
        .await
        .expect("Failed to set output path");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateNotifyRequest {
    live_start_notify: bool,
    live_end_notify: bool,
    clip_notify: bool,
    post_notify: bool,
}

async fn handler_update_notify(
    state: axum::extract::State<State>,
    Json(notify): Json<UpdateNotifyRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_notify(
        state.0,
        notify.live_start_notify,
        notify.live_end_notify,
        notify.clip_notify,
        notify.post_notify,
    )
    .await
    .expect("Failed to update notify");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWhisperModelRequest {
    whisper_model: String,
}

async fn handler_update_whisper_model(
    state: axum::extract::State<State>,
    Json(whisper_model): Json<UpdateWhisperModelRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_whisper_model(state.0, whisper_model.whisper_model)
        .await
        .expect("Failed to update whisper model");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSubtitleSettingRequest {
    auto_subtitle: bool,
}

async fn handler_update_subtitle_setting(
    state: axum::extract::State<State>,
    Json(subtitle_setting): Json<UpdateSubtitleSettingRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_subtitle_setting(state.0, subtitle_setting.auto_subtitle)
        .await
        .expect("Failed to update subtitle setting");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateClipNameFormatRequest {
    clip_name_format: String,
}

async fn handler_update_clip_name_format(
    state: axum::extract::State<State>,
    Json(clip_name_format): Json<UpdateClipNameFormatRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_clip_name_format(state.0, clip_name_format.clip_name_format)
        .await
        .expect("Failed to update clip name format");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWhisperPromptRequest {
    whisper_prompt: String,
}

async fn handler_update_whisper_prompt(
    state: axum::extract::State<State>,
    Json(whisper_prompt): Json<UpdateWhisperPromptRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_whisper_prompt(state.0, whisper_prompt.whisper_prompt)
        .await
        .expect("Failed to update whisper prompt");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAutoGenerateRequest {
    enable: bool,
    encode_danmu: bool,
}

async fn handler_update_auto_generate(
    state: axum::extract::State<State>,
    Json(auto_generate): Json<UpdateAutoGenerateRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_auto_generate(state.0, auto_generate.enable, auto_generate.encode_danmu)
        .await
        .expect("Failed to update auto generate");
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_messages(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<MessageRow>>>, String> {
    let messages = get_messages(state.0).await.expect("Failed to get messages");
    Ok(Json(ApiResponse::success(messages)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadMessageRequest {
    message_id: i64,
}

async fn handler_read_message(
    state: axum::extract::State<State>,
    Json(message): Json<ReadMessageRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    read_message(state.0, message.message_id)
        .await
        .expect("Failed to read message");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteMessageRequest {
    message_id: i64,
}

async fn handler_delete_message(
    state: axum::extract::State<State>,
    Json(message): Json<DeleteMessageRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    delete_message(state.0, message.message_id)
        .await
        .expect("Failed to delete message");
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_recorder_list(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<RecorderList>>, String> {
    let recorders = get_recorder_list(state.0)
        .await
        .expect("Failed to get recorder list");
    Ok(Json(ApiResponse::success(recorders)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddRecorderRequest {
    platform: String,
    room_id: u64,
}

async fn handler_add_recorder(
    state: axum::extract::State<State>,
    Json(param): Json<AddRecorderRequest>,
) -> Result<Json<ApiResponse<RecorderRow>>, String> {
    let recorder = add_recorder(state.0, param.platform, param.room_id)
        .await
        .expect("Failed to add recorder");
    Ok(Json(ApiResponse::success(recorder)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveRecorderRequest {
    platform: String,
    room_id: u64,
}

async fn handler_remove_recorder(
    state: axum::extract::State<State>,
    Json(param): Json<RemoveRecorderRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    remove_recorder(state.0, param.platform, param.room_id)
        .await
        .expect("Failed to remove recorder");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRoomInfoRequest {
    platform: String,
    room_id: u64,
}

async fn handler_get_room_info(
    state: axum::extract::State<State>,
    Json(param): Json<GetRoomInfoRequest>,
) -> Result<Json<ApiResponse<RecorderInfo>>, String> {
    let room_info = get_room_info(state.0, param.platform, param.room_id).await?;
    Ok(Json(ApiResponse::success(room_info)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchivesRequest {
    room_id: u64,
}

async fn handler_get_archives(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchivesRequest>,
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, String> {
    let archives = get_archives(state.0, param.room_id).await?;
    Ok(Json(ApiResponse::success(archives)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchiveRequest {
    room_id: u64,
    live_id: String,
}

async fn handler_get_archive(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchiveRequest>,
) -> Result<Json<ApiResponse<RecordRow>>, String> {
    let archive = get_archive(state.0, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(archive)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteArchiveRequest {
    platform: String,
    room_id: u64,
    archive_id: String,
}

async fn handler_delete_archive(
    state: axum::extract::State<State>,
    Json(param): Json<DeleteArchiveRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    delete_archive(state.0, param.platform, param.room_id, param.archive_id).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetDanmuRecordRequest {
    platform: String,
    room_id: u64,
    live_id: String,
}

async fn handler_get_danmu_record(
    state: axum::extract::State<State>,
    Json(param): Json<GetDanmuRecordRequest>,
) -> Result<Json<ApiResponse<Vec<DanmuEntry>>>, String> {
    let danmu_record =
        get_danmu_record(state.0, param.platform, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(danmu_record)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendDanmakuRequest {
    uid: u64,
    room_id: u64,
    message: String,
}

async fn handler_send_danmaku(
    state: axum::extract::State<State>,
    Json(param): Json<SendDanmakuRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    send_danmaku(state.0, param.uid, param.room_id, param.message).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_total_length(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<i64>>, String> {
    let total_length = get_total_length(state.0).await?;
    Ok(Json(ApiResponse::success(total_length)))
}

async fn handler_get_today_record_count(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<i64>>, String> {
    let today_record_count = get_today_record_count(state.0).await?;
    Ok(Json(ApiResponse::success(today_record_count)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRecentRecordRequest {
    offset: u64,
    limit: u64,
}

async fn handler_get_recent_record(
    state: axum::extract::State<State>,
    Json(param): Json<GetRecentRecordRequest>,
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, String> {
    let recent_record = get_recent_record(state.0, param.offset, param.limit).await?;
    Ok(Json(ApiResponse::success(recent_record)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetAutoStartRequest {
    platform: String,
    room_id: u64,
    auto_start: bool,
}
async fn handler_set_auto_start(
    state: axum::extract::State<State>,
    Json(param): Json<SetAutoStartRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    set_auto_start(state.0, param.platform, param.room_id, param.auto_start).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForceStartRequest {
    platform: String,
    room_id: u64,
}

async fn handler_force_start(
    state: axum::extract::State<State>,
    Json(param): Json<ForceStartRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    force_start(state.0, param.platform, param.room_id).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForceStopRequest {
    platform: String,
    room_id: u64,
}

async fn handler_force_stop(
    state: axum::extract::State<State>,
    Json(param): Json<ForceStopRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    force_stop(state.0, param.platform, param.room_id).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipRangeRequest {
    event_id: String,
    params: ClipRangeParams,
}

async fn handler_clip_range(
    state: axum::extract::State<State>,
    Json(param): Json<ClipRangeRequest>,
) -> Result<Json<ApiResponse<String>>, String> {
    clip_range(state.0, param.event_id.clone(), param.params).await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadProcedureRequest {
    uid: u64,
    room_id: u64,
    video_id: i64,
    cover: String,
    profile: Profile,
}

async fn handler_upload_procedure(
    state: axum::extract::State<State>,
    Json(param): Json<UploadProcedureRequest>,
) -> Result<Json<ApiResponse<String>>, String> {
    let event_id = Uuid::new_v4().to_string();
    upload_procedure(
        state.0,
        event_id.clone(),
        param.uid,
        param.room_id,
        param.video_id,
        param.cover,
        param.profile,
    )
    .await?;
    Ok(Json(ApiResponse::success(event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    event_id: String,
}

async fn handler_cancel(
    state: axum::extract::State<State>,
    Json(param): Json<CancelRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    cancel(state.0, param.event_id).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideoRequest {
    id: i64,
}

async fn handler_get_video(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideoRequest>,
) -> Result<Json<ApiResponse<VideoRow>>, String> {
    let video = get_video(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(video)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideosRequest {
    room_id: u64,
}
async fn handler_get_videos(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideosRequest>,
) -> Result<Json<ApiResponse<Vec<VideoRow>>>, String> {
    let videos = get_videos(state.0, param.room_id).await?;
    Ok(Json(ApiResponse::success(videos)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteVideoRequest {
    id: i64,
}

async fn handler_delete_video(
    state: axum::extract::State<State>,
    Json(param): Json<DeleteVideoRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    delete_video(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_video_typelist(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<Typelist>>>, String> {
    let video_typelist = get_video_typelist(state.0).await?;
    Ok(Json(ApiResponse::success(video_typelist)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateVideoCoverRequest {
    id: i64,
    cover: String,
}

async fn handler_update_video_cover(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateVideoCoverRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_video_cover(state.0, param.id, param.cover).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateVideoSubtitleRequest {
    id: i64,
}

async fn handler_generate_video_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GenerateVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, String> {
    let uuid = Uuid::new_v4().to_string();
    generate_video_subtitle(state.0, uuid.clone(), param.id).await?;
    Ok(Json(ApiResponse::success(uuid)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideoSubtitleRequest {
    id: i64,
}

async fn handler_get_video_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, String> {
    let video_subtitle = get_video_subtitle(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(video_subtitle)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateVideoSubtitleRequest {
    id: i64,
    subtitle: String,
}

async fn handler_update_video_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<()>>, String> {
    update_video_subtitle(state.0, param.id, param.subtitle).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncodeVideoSubtitleRequest {
    id: i64,
    srt_style: String,
}

async fn handler_encode_video_subtitle(
    state: axum::extract::State<State>,
    Json(encode_video_subtitle_param): Json<EncodeVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, String> {
    // generate uuid
    let uuid = Uuid::new_v4().to_string();
    encode_video_subtitle(
        state.0,
        uuid.clone(),
        encode_video_subtitle_param.id,
        encode_video_subtitle_param.srt_style,
    )
    .await?;
    Ok(Json(ApiResponse::success(uuid)))
}
async fn handler_get_disk_info(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<DiskInfo>>, String> {
    let disk_info = get_disk_info(state.0)
        .await
        .map_err(|_| "Failed to get disk info")?;
    Ok(Json(ApiResponse::success(disk_info)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpProxyRequest {
    url: String,
    method: String,
    headers: Option<std::collections::HashMap<String, String>>,
    body: Option<String>,
}

async fn handler_fetch(
    _state: axum::extract::State<State>,
    Json(param): Json<HttpProxyRequest>,
) -> Result<impl IntoResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = match param.method.to_uppercase().as_str() {
        "GET" => client.get(&param.url),
        "POST" => client.post(&param.url),
        "PUT" => client.put(&param.url),
        "DELETE" => client.delete(&param.url),
        "PATCH" => client.patch(&param.url),
        _ => return Err("Unsupported HTTP method".to_string()),
    };

    // Add headers if present
    if let Some(headers) = param.headers {
        for (key, value) in headers {
            request = request.header(key, value);
        }
    }

    // Add body if present
    if let Some(body) = param.body {
        request = request.body(body);
    }

    let response = request.send().await.map_err(|e| e.to_string())?;

    let status = axum::http::StatusCode::from_u16(response.status().as_u16())
        .map_err(|_| "Invalid status code".to_string())?;
    let headers = response.headers().clone();

    // Get response body
    let body = response.bytes().await.map_err(|e| e.to_string())?;

    // Create response headers
    let mut response_headers = axum::http::HeaderMap::new();
    for (key, value) in headers.iter() {
        if let Ok(value_str) = value.to_str() {
            if let Ok(header_name) = axum::http::HeaderName::from_bytes(key.as_ref()) {
                if let Ok(header_value) = axum::http::HeaderValue::from_str(value_str) {
                    response_headers.insert(header_name, header_value);
                }
            }
        }
    }

    Ok((status, response_headers, body))
}

async fn handler_hls(
    state: axum::extract::State<State>,
    Path(uri): Path<String>,
    query: Option<Query<std::collections::HashMap<String, String>>>,
) -> Result<impl IntoResponse, StatusCode> {
    let path_segs: Vec<&str> = uri.split('/').collect();

    if path_segs.len() < 4 {
        return Err(StatusCode::NOT_FOUND);
    }

    let filename = path_segs[3];

    let query_str = query
        .map(|q| {
            q.0.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
                .join("&")
        })
        .unwrap_or_default();
    let uri_with_query = format!("{}?{}", uri, query_str);

    let hls = fetch_hls(state.0, uri_with_query)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    // Set appropriate content type based on file extension
    let content_type = match filename.split('.').last() {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("ts") => "video/mp2t",
        Some("aac") => "audio/aac",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("m4s") => "video/iso.segment",
        _ => "application/octet-stream",
    };

    // Create response with necessary headers
    let mut response =
        axum::response::Response::<axum::body::Body>::new(axum::body::Body::from(hls));
    let headers = response.headers_mut();

    // Set content type
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );

    // Only set cache control for m3u8 files
    if filename.ends_with(".m3u8") {
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            "no-cache, no-store, must-revalidate".parse().unwrap(),
        );
        headers.insert(axum::http::header::PRAGMA, "no-cache".parse().unwrap());
        headers.insert(axum::http::header::EXPIRES, "0".parse().unwrap());
    }

    Ok(response)
}

async fn handler_output(
    state: axum::extract::State<State>,
    headers: axum::http::HeaderMap,
    Path(uri): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate path and get file
    if uri.contains("..") {
        return Err(StatusCode::NOT_FOUND);
    }
    let output_path = state.config.read().await.output.clone();
    let path = std::path::Path::new(&output_path).join(uri);
    if !path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let metadata = file.metadata().await.map_err(|_| StatusCode::NOT_FOUND)?;
    let file_size = metadata.len();

    // Parse range header if present
    let range_header = headers.get(axum::http::header::RANGE);
    let (start, end) = if let Some(range) = range_header {
        let range_str = range.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
        let range = range_str
            .strip_prefix("bytes=")
            .ok_or(StatusCode::BAD_REQUEST)?;
        let parts: Vec<&str> = range.split('-').collect();
        if parts.len() != 2 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let start = parts[0]
            .parse::<u64>()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let end = if parts[1].is_empty() {
            file_size - 1
        } else {
            parts[1]
                .parse::<u64>()
                .map_err(|_| StatusCode::BAD_REQUEST)?
        };
        if start > end || end >= file_size {
            return Err(StatusCode::RANGE_NOT_SATISFIABLE);
        }
        (start, end)
    } else {
        (0, file_size - 1)
    };

    // Seek to the start position
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create a stream for the requested range
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    // Create response with appropriate headers
    let mut response = axum::response::Response::new(body);

    // Set content type based on file extension
    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("m4v") => "video/x-m4v",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        _ => "application/octet-stream",
    };

    // Set headers
    {
        let headers = response.headers_mut();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            content_type.parse().unwrap(),
        );

        let content_length = end - start + 1;
        headers.insert(
            axum::http::header::CONTENT_LENGTH,
            content_length.to_string().parse().unwrap(),
        );

        headers.insert(axum::http::header::ACCEPT_RANGES, "bytes".parse().unwrap());

        // Set partial content status and range headers if needed
        if range_header.is_some() {
            headers.insert(
                axum::http::header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", start, end, file_size)
                    .parse()
                    .unwrap(),
            );
        }
    }

    if range_header.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }

    Ok(response)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerEvent {
    event: String,
    data: String,
}

async fn handler_sse(
    state: axum::extract::State<State>,
) -> Sse<impl Stream<Item = Result<sse::Event, axum::Error>>> {
    let rx = state.progress_manager.subscribe();

    let stream = stream::unfold(rx, move |mut rx| async move {
        match rx.recv().await {
            Ok(event) => {
                let event = match event {
                    Event::ProgressUpdate { id, content } => sse::Event::default()
                        .event("progress-update")
                        .data(format!(r#"{{"id":"{}","content":"{}"}}"#, id, content)),
                    Event::ProgressFinished {
                        id,
                        success,
                        message,
                    } => sse::Event::default()
                        .event("progress-finished")
                        .data(format!(
                            r#"{{"id":"{}","success":{},"message":"{}"}}"#,
                            id, success, message
                        )),
                    Event::DanmuReceived { room, ts, content } => sse::Event::default()
                        .event(format!("danmu:{}", room))
                        .data(format!(r#"{{"ts":"{}","content":"{}"}}"#, ts, content)),
                };
                Some((Ok(event), rx))
            }
            Err(_) => None,
        }
    });

    Sse::new(stream).keep_alive(
        sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(1))
            .text("keep-alive"),
    )
}

pub async fn start_api_server(state: State) {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Configure body size limit
    let body_limit = tower_http::limit::RequestBodyLimitLayer::new(1024 * 1024 * 1024); // 1GB limit

    let app = Router::new()
        // Serve static files from dist directory
        .nest_service("/", ServeDir::new("../dist"))
        // Account commands
        .route("/api/get_accounts", post(handler_get_accounts))
        .route("/api/add_account", post(handler_add_account))
        .route("/api/remove_account", post(handler_remove_account))
        .route("/api/get_account_count", post(handler_get_account_count))
        .route("/api/get_qr", post(handler_get_qr))
        .route("/api/get_qr_status", post(handler_get_qr_status))
        // Config commands
        .route("/api/get_config", post(handler_get_config))
        .route("/api/set_cache_path", post(handler_set_cache_path))
        .route("/api/set_output_path", post(handler_set_output_path))
        .route("/api/update_notify", post(handler_update_notify))
        .route(
            "/api/update_whisper_model",
            post(handler_update_whisper_model),
        )
        .route(
            "/api/update_subtitle_setting",
            post(handler_update_subtitle_setting),
        )
        .route(
            "/api/update_clip_name_format",
            post(handler_update_clip_name_format),
        )
        .route(
            "/api/update_whisper_prompt",
            post(handler_update_whisper_prompt),
        )
        .route(
            "/api/update_auto_generate",
            post(handler_update_auto_generate),
        )
        // Message commands
        .route("/api/get_messages", post(handler_get_messages))
        .route("/api/read_message", post(handler_read_message))
        .route("/api/delete_message", post(handler_delete_message))
        // Recorder commands
        .route("/api/get_recorder_list", post(handler_get_recorder_list))
        .route("/api/add_recorder", post(handler_add_recorder))
        .route("/api/remove_recorder", post(handler_remove_recorder))
        .route("/api/get_room_info", post(handler_get_room_info))
        .route("/api/get_archives", post(handler_get_archives))
        .route("/api/get_archive", post(handler_get_archive))
        .route("/api/delete_archive", post(handler_delete_archive))
        .route("/api/get_danmu_record", post(handler_get_danmu_record))
        .route("/api/send_danmaku", post(handler_send_danmaku))
        .route("/api/get_total_length", post(handler_get_total_length))
        .route(
            "/api/get_today_record_count",
            post(handler_get_today_record_count),
        )
        .route("/api/get_recent_record", post(handler_get_recent_record))
        .route("/api/set_auto_start", post(handler_set_auto_start))
        .route("/api/force_start", post(handler_force_start))
        .route("/api/force_stop", post(handler_force_stop))
        // Video commands
        .route("/api/clip_range", post(handler_clip_range))
        .route("/api/upload_procedure", post(handler_upload_procedure))
        .route("/api/cancel", post(handler_cancel))
        .route("/api/get_video", post(handler_get_video))
        .route("/api/get_videos", post(handler_get_videos))
        .route("/api/delete_video", post(handler_delete_video))
        .route("/api/get_video_typelist", post(handler_get_video_typelist))
        .route("/api/update_video_cover", post(handler_update_video_cover))
        .route(
            "/api/generate_video_subtitle",
            post(handler_generate_video_subtitle),
        )
        .route("/api/get_video_subtitle", post(handler_get_video_subtitle))
        .route(
            "/api/update_video_subtitle",
            post(handler_update_video_subtitle),
        )
        .route(
            "/api/encode_video_subtitle",
            post(handler_encode_video_subtitle),
        )
        // Utils commands
        .route("/api/get_disk_info", post(handler_get_disk_info))
        .route("/api/fetch", post(handler_fetch))
        .route("/hls/*uri", get(handler_hls))
        .route("/output/*uri", get(handler_output))
        .route("/api/sse", get(handler_sse))
        .layer(cors)
        .layer(body_limit)
        .with_state(state);

    let addr = "0.0.0.0:3000";
    println!("API server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
