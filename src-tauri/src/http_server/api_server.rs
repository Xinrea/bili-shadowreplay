use std::{
    fmt::{self, Display},
    path::PathBuf,
};

use crate::{
    config::Config,
    database::{
        account::AccountRow, message::MessageRow, record::RecordRow, recorder::RecorderRow,
        task::TaskRow, video::VideoRow,
    },
    handlers::{
        account::{
            add_account, get_account_count, get_accounts, get_qr, get_qr_status, remove_account,
        },
        config::{
            get_config, update_auto_generate, update_clip_name_format, update_notify,
            update_openai_api_endpoint, update_openai_api_key, update_status_check_interval,
            update_subtitle_generator_type, update_subtitle_setting, update_webhook_url,
            update_whisper_language, update_whisper_model, update_whisper_prompt,
        },
        message::{delete_message, get_messages, read_message},
        recorder::{
            add_recorder, delete_archive, delete_archives, export_danmu, fetch_hls,
            generate_archive_subtitle, generate_whole_clip, get_archive, get_archive_disk_usage,
            get_archive_subtitle, get_archives, get_archives_by_parent_id, get_danmu_record,
            get_recent_record, get_recorder_list, get_room_info, get_today_record_count,
            get_total_length, remove_recorder, send_danmaku, set_enable, ExportDanmuOptions,
        },
        task::{delete_task, get_tasks},
        utils::{console_log, get_disk_info, list_folder, sanitize_filename_advanced, DiskInfo},
        video::{
            batch_import_external_videos, cancel, clip_range, clip_video, delete_video,
            encode_video_subtitle, generate_video_subtitle, generic_ffmpeg_command, get_all_videos,
            get_file_size, get_import_progress, get_video, get_video_cover, get_video_subtitle,
            get_video_typelist, get_videos, import_external_video, update_video_cover,
            update_video_note, update_video_subtitle, upload_procedure,
        },
        AccountInfo,
    },
    http_server::websocket,
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
use axum::extract::Query;
use axum::{
    extract::{DefaultBodyLimit, Json, Multipart, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

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

#[derive(Debug)]
struct ApiError(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        Json(ApiResponse::<()>::error(self.0)).into_response()
    }
}

impl From<String> for ApiError {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ApiError {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

async fn handler_get_accounts(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<AccountInfo>>, ApiError> {
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
) -> Result<Json<ApiResponse<AccountRow>>, ApiError> {
    let mut account = add_account(state.0, param.platform, &param.cookies).await?;
    account.cookies = "".to_string();
    Ok(Json(ApiResponse::success(account)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveAccountRequest {
    platform: String,
    uid: i64,
}

async fn handler_remove_account(
    state: axum::extract::State<State>,
    Json(account): Json<RemoveAccountRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    remove_account(state.0, account.platform, account.uid).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_account_count(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<u64>>, ApiError> {
    let count = get_account_count(state.0).await?;
    Ok(Json(ApiResponse::success(count)))
}

async fn handler_get_qr(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<QrInfo>>, ApiError> {
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
) -> Result<Json<ApiResponse<QrStatus>>, ApiError> {
    let qr_status = get_qr_status(state.0, &qr_info.qrcode_key)
        .await
        .expect("Failed to get QR status");
    Ok(Json(ApiResponse::success(qr_status)))
}

async fn handler_get_config(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Config>>, ApiError> {
    let config = get_config(state.0).await.expect("Failed to get config");
    Ok(Json(ApiResponse::success(config)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatusCheckIntervalRequest {
    interval: u64,
}

async fn handler_update_status_check_interval(
    state: axum::extract::State<State>,
    Json(request): Json<UpdateStatusCheckIntervalRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_status_check_interval(state.0, request.interval)
        .await
        .expect("Failed to update status check interval");
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_whisper_model(state.0, whisper_model.whisper_model)
        .await
        .expect("Failed to update whisper model");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWhisperLanguageRequest {
    whisper_language: String,
}

async fn handler_update_whisper_language(
    state: axum::extract::State<State>,
    Json(whisper_language): Json<UpdateWhisperLanguageRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_whisper_language(state.0, whisper_language.whisper_language)
        .await
        .expect("Failed to update whisper language");
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_whisper_prompt(state.0, whisper_prompt.whisper_prompt)
        .await
        .expect("Failed to update whisper prompt");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWebhookUrlRequest {
    webhook_url: String,
}

async fn handler_update_webhook_url(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateWebhookUrlRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_webhook_url(state.0, param.webhook_url)
        .await
        .expect("Failed to update webhook url");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSubtitleGeneratorTypeRequest {
    subtitle_generator_type: String,
}

async fn handler_update_subtitle_generator_type(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateSubtitleGeneratorTypeRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_subtitle_generator_type(state.0, param.subtitle_generator_type)
        .await
        .expect("Failed to update subtitle generator type");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateOpenaiApiEndpointRequest {
    openai_api_endpoint: String,
}

async fn handler_update_openai_api_endpoint(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateOpenaiApiEndpointRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_openai_api_endpoint(state.0, param.openai_api_endpoint)
        .await
        .expect("Failed to update openai api endpoint");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateOpenaiApiKeyRequest {
    openai_api_key: String,
}

async fn handler_update_openai_api_key(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateOpenaiApiKeyRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_openai_api_key(state.0, param.openai_api_key)
        .await
        .expect("Failed to update openai api key");
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_auto_generate(state.0, auto_generate.enable, auto_generate.encode_danmu)
        .await
        .expect("Failed to update auto generate");
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_messages(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<MessageRow>>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    delete_message(state.0, message.message_id)
        .await
        .expect("Failed to delete message");
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_recorder_list(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<RecorderList>>, ApiError> {
    let recorders = get_recorder_list(state.0)
        .await
        .expect("Failed to get recorder list");
    Ok(Json(ApiResponse::success(recorders)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddRecorderRequest {
    platform: String,
    room_id: i64,
    extra: String,
}

async fn handler_add_recorder(
    state: axum::extract::State<State>,
    Json(param): Json<AddRecorderRequest>,
) -> Result<Json<ApiResponse<RecorderRow>>, ApiError> {
    let recorder = add_recorder(state.0, param.platform, param.room_id, param.extra)
        .await
        .expect("Failed to add recorder");
    Ok(Json(ApiResponse::success(recorder)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveRecorderRequest {
    platform: String,
    room_id: i64,
}

async fn handler_remove_recorder(
    state: axum::extract::State<State>,
    Json(param): Json<RemoveRecorderRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    remove_recorder(state.0, param.platform, param.room_id)
        .await
        .expect("Failed to remove recorder");
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRoomInfoRequest {
    platform: String,
    room_id: i64,
}

async fn handler_get_room_info(
    state: axum::extract::State<State>,
    Json(param): Json<GetRoomInfoRequest>,
) -> Result<Json<ApiResponse<RecorderInfo>>, ApiError> {
    let room_info = get_room_info(state.0, param.platform, param.room_id).await?;
    Ok(Json(ApiResponse::success(room_info)))
}

async fn handler_get_archive_disk_usage(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<i64>>, ApiError> {
    let disk_usage = get_archive_disk_usage(state.0).await?;
    Ok(Json(ApiResponse::success(disk_usage)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchivesRequest {
    room_id: i64,
    offset: i64,
    limit: i64,
}

async fn handler_get_archives(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchivesRequest>,
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, ApiError> {
    let archives = get_archives(state.0, param.room_id, param.offset, param.limit).await?;
    Ok(Json(ApiResponse::success(archives)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchiveRequest {
    room_id: i64,
    live_id: String,
}

async fn handler_get_archive(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchiveRequest>,
) -> Result<Json<ApiResponse<RecordRow>>, ApiError> {
    let archive = get_archive(state.0, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(archive)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchiveSubtitleRequest {
    platform: String,
    room_id: i64,
    live_id: String,
}

async fn handler_get_archive_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchiveSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let subtitle =
        get_archive_subtitle(state.0, param.platform, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(subtitle)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateArchiveSubtitleRequest {
    platform: String,
    room_id: i64,
    live_id: String,
}

async fn handler_generate_archive_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GenerateArchiveSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let subtitle =
        generate_archive_subtitle(state.0, param.platform, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(subtitle)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteArchiveRequest {
    platform: String,
    room_id: i64,
    live_id: String,
}

async fn handler_delete_archive(
    state: axum::extract::State<State>,
    Json(param): Json<DeleteArchiveRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    delete_archive(state.0, param.platform, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteArchivesRequest {
    platform: String,
    room_id: i64,
    live_ids: Vec<String>,
}

async fn handler_delete_archives(
    state: axum::extract::State<State>,
    Json(param): Json<DeleteArchivesRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    delete_archives(state.0, param.platform, param.room_id, param.live_ids).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetDanmuRecordRequest {
    platform: String,
    room_id: i64,
    live_id: String,
}

async fn handler_get_danmu_record(
    state: axum::extract::State<State>,
    Json(param): Json<GetDanmuRecordRequest>,
) -> Result<Json<ApiResponse<Vec<DanmuEntry>>>, ApiError> {
    let danmu_record =
        get_danmu_record(state.0, param.platform, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(danmu_record)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendDanmakuRequest {
    uid: i64,
    room_id: i64,
    message: String,
}

async fn handler_send_danmaku(
    state: axum::extract::State<State>,
    Json(param): Json<SendDanmakuRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    send_danmaku(state.0, param.uid, param.room_id, param.message).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_total_length(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<i64>>, ApiError> {
    let total_length = get_total_length(state.0).await?;
    Ok(Json(ApiResponse::success(total_length)))
}

async fn handler_get_today_record_count(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<i64>>, ApiError> {
    let today_record_count = get_today_record_count(state.0).await?;
    Ok(Json(ApiResponse::success(today_record_count)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRecentRecordRequest {
    room_id: i64,
    offset: i64,
    limit: i64,
}

async fn handler_get_recent_record(
    state: axum::extract::State<State>,
    Json(param): Json<GetRecentRecordRequest>,
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, ApiError> {
    let recent_record =
        get_recent_record(state.0, param.room_id, param.offset, param.limit).await?;
    Ok(Json(ApiResponse::success(recent_record)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetEnableRequest {
    platform: String,
    room_id: i64,
    enabled: bool,
}

async fn handler_set_enable(
    state: axum::extract::State<State>,
    Json(param): Json<SetEnableRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    set_enable(state.0, param.platform, param.room_id, param.enabled).await?;
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
) -> Result<Json<ApiResponse<String>>, ApiError> {
    clip_range(state.0, param.event_id.clone(), param.params).await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadProcedureRequest {
    event_id: String,
    uid: i64,
    room_id: i64,
    video_id: i64,
    cover: String,
    profile: Profile,
}

async fn handler_upload_procedure(
    state: axum::extract::State<State>,
    Json(param): Json<UploadProcedureRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    upload_procedure(
        state.0,
        param.event_id.clone(),
        param.uid,
        param.room_id,
        param.video_id,
        param.cover,
        param.profile,
    )
    .await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    event_id: String,
}

async fn handler_cancel(
    state: axum::extract::State<State>,
    Json(param): Json<CancelRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
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
) -> Result<Json<ApiResponse<VideoRow>>, ApiError> {
    let video = get_video(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(video)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideosRequest {
    room_id: i64,
}

async fn handler_get_videos(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideosRequest>,
) -> Result<Json<ApiResponse<Vec<VideoRow>>>, ApiError> {
    let videos = get_videos(state.0, param.room_id).await?;
    Ok(Json(ApiResponse::success(videos)))
}

async fn handler_get_all_videos(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<VideoRow>>>, ApiError> {
    let videos = get_all_videos(state.0).await?;
    Ok(Json(ApiResponse::success(videos)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideoCoverRequest {
    id: i64,
}

async fn handler_get_video_cover(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideoCoverRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let video_cover = get_video_cover(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(video_cover)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteVideoRequest {
    id: i64,
}

async fn handler_delete_video(
    state: axum::extract::State<State>,
    Json(param): Json<DeleteVideoRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    delete_video(state.0, param.id).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_video_typelist(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<Typelist>>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_video_cover(state.0, param.id, param.cover).await?;
    Ok(Json(ApiResponse::success(())))
}

// 处理base64图片数据的API
async fn handler_image_base64(
    Path(video_id): Path<i64>,
    state: axum::extract::State<State>,
) -> Result<impl IntoResponse, StatusCode> {
    // 获取视频封面
    let cover = match get_video_cover(state.0, video_id).await {
        Ok(cover) => cover,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };

    // 检查是否是base64数据URL
    if cover.starts_with("data:image/") {
        if let Some(base64_start) = cover.find("base64,") {
            let base64_data = &cover[base64_start + 7..]; // 跳过 "base64,"

            // 解码base64数据
            use base64::{engine::general_purpose, Engine as _};
            if let Ok(image_data) = general_purpose::STANDARD.decode(base64_data) {
                // 确定MIME类型
                let content_type = if cover.contains("data:image/png") {
                    "image/png"
                } else if cover.contains("data:image/jpeg") || cover.contains("data:image/jpg") {
                    "image/jpeg"
                } else if cover.contains("data:image/gif") {
                    "image/gif"
                } else if cover.contains("data:image/webp") {
                    "image/webp"
                } else {
                    "image/png" // 默认
                };

                let mut response =
                    axum::response::Response::new(axum::body::Body::from(image_data));
                let headers = response.headers_mut();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    content_type.parse().unwrap(),
                );
                headers.insert(
                    axum::http::header::CACHE_CONTROL,
                    "public, max-age=3600".parse().unwrap(),
                );

                return Ok(response);
            }
        }
    }

    Err(StatusCode::NOT_FOUND)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateVideoSubtitleRequest {
    event_id: String,
    id: i64,
}

async fn handler_generate_video_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GenerateVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let result = generate_video_subtitle(state.0, param.event_id.clone(), param.id).await?;
    Ok(Json(ApiResponse::success(result)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetVideoSubtitleRequest {
    id: i64,
}

async fn handler_get_video_subtitle(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
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
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_video_subtitle(state.0, param.id, param.subtitle).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateVideoNoteRequest {
    id: i64,
    note: String,
}

async fn handler_update_video_note(
    state: axum::extract::State<State>,
    Json(param): Json<UpdateVideoNoteRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_video_note(state.0, param.id, param.note).await?;
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncodeVideoSubtitleRequest {
    event_id: String,
    id: i64,
    srt_style: String,
}

async fn handler_encode_video_subtitle(
    state: axum::extract::State<State>,
    Json(encode_video_subtitle_param): Json<EncodeVideoSubtitleRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    encode_video_subtitle(
        state.0,
        encode_video_subtitle_param.event_id.clone(),
        encode_video_subtitle_param.id,
        encode_video_subtitle_param.srt_style,
    )
    .await?;
    Ok(Json(ApiResponse::success(
        encode_video_subtitle_param.event_id,
    )))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportExternalVideoRequest {
    event_id: String,
    file_path: String,
    title: String,
    room_id: i64,
}

async fn handler_import_external_video(
    state: axum::extract::State<State>,
    Json(param): Json<ImportExternalVideoRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    import_external_video(
        state.0,
        param.event_id.clone(),
        param.file_path.clone(),
        param.title,
        param.room_id,
    )
    .await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipVideoRequest {
    event_id: String,
    parent_video_id: i64,
    start_time: f64,
    end_time: f64,
    clip_title: String,
}

async fn handler_clip_video(
    state: axum::extract::State<State>,
    Json(param): Json<ClipVideoRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    clip_video(
        state.0,
        param.event_id.clone(),
        param.parent_video_id,
        param.start_time,
        param.end_time,
        param.clip_title,
    )
    .await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFileSizeRequest {
    file_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateWholeClipRequest {
    platform: String,
    room_id: i64,
    parent_id: String,
}

async fn handler_generate_whole_clip(
    state: axum::extract::State<State>,
    Json(param): Json<GenerateWholeClipRequest>,
) -> Result<Json<ApiResponse<TaskRow>>, ApiError> {
    let task = generate_whole_clip(state.0, param.platform, param.room_id, param.parent_id).await?;
    Ok(Json(ApiResponse::success(task)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchivesByParentIdRequest {
    room_id: i64,
    parent_id: String,
}

async fn handler_get_archives_by_parent_id(
    state: axum::extract::State<State>,
    Json(param): Json<GetArchivesByParentIdRequest>,
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, ApiError> {
    let archives = get_archives_by_parent_id(state.0, param.room_id, param.parent_id).await?;
    Ok(Json(ApiResponse::success(archives)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchImportExternalVideosRequest {
    event_id: String,
    file_paths: Vec<String>,
    room_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportProgressResponse {
    task_id: Option<String>,
    file_name: Option<String>,
    file_size: Option<u64>,
    message: Option<String>,
    status: Option<String>,
    created_at: Option<String>,
}

async fn handler_get_file_size(
    _state: axum::extract::State<State>,
    Json(param): Json<GetFileSizeRequest>,
) -> Result<Json<ApiResponse<u64>>, ApiError> {
    let file_size = get_file_size(param.file_path).await?;
    Ok(Json(ApiResponse::success(file_size)))
}

async fn handler_batch_import_external_videos(
    state: axum::extract::State<State>,
    Json(param): Json<BatchImportExternalVideosRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    batch_import_external_videos(
        state.0,
        param.event_id.clone(),
        param.file_paths,
        param.room_id,
    )
    .await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

async fn handler_get_import_progress(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Option<ImportProgressResponse>>>, ApiError> {
    let progress = get_import_progress(state.0).await?;

    if let Some(progress_data) = progress {
        let response = ImportProgressResponse {
            task_id: progress_data
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            file_name: progress_data
                .get("file_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            file_size: progress_data.get("file_size").and_then(|v| v.as_u64()),
            message: progress_data
                .get("message")
                .and_then(|v| v.as_str())
                .map(String::from),
            status: progress_data
                .get("status")
                .and_then(|v| v.as_str())
                .map(String::from),
            created_at: progress_data
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        Ok(Json(ApiResponse::success(Some(response))))
    } else {
        Ok(Json(ApiResponse::success(None)))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadFilesResponse {
    uploaded_files: Vec<UploadedFileInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadedFileInfo {
    file_path: String,
    original_name: String,
    size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadAndImportResponse {
    event_id: String,
    uploaded_files: Vec<UploadedFileInfo>,
}

// 多文件上传处理器
async fn handler_upload_files(
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<UploadFilesResponse>>, ApiError> {
    let mut uploaded_files = Vec::new();
    let upload_dir = std::env::temp_dir().join("bsr_uploads");

    // 确保上传目录存在
    if !upload_dir.exists() {
        std::fs::create_dir_all(&upload_dir).map_err(|e| format!("创建上传目录失败: {}", e))?;
    }

    while let Some(mut field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        if let Some(file_name) = field.file_name() {
            let file_name = file_name.to_string();

            // 检查文件格式是否为支持的视频格式
            let extension = std::path::Path::new(&file_name)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            // 使用与后端相同的格式验证逻辑
            let supported_extensions = ["mp4", "mkv", "avi", "mov", "wmv", "flv", "m4v", "webm"];
            if !supported_extensions.iter().any(|&ext| ext == extension) {
                return Err(ApiError(format!(
                    "不支持的文件格式: {}。支持的格式: {}",
                    extension,
                    supported_extensions.join(", ")
                )));
            }

            // 生成唯一的文件名
            let timestamp = chrono::Utc::now().timestamp();
            let sanitized_name = sanitize_filename_advanced(&file_name, None);
            let unique_name = format!("{}_{}", timestamp, sanitized_name);
            let file_path = upload_dir.join(&unique_name);

            // 流式保存文件，避免大文件内存占用
            let mut file = tokio::fs::File::create(&file_path)
                .await
                .map_err(|e| format!("创建文件失败: {}", e))?;

            let mut total_size = 0u64;
            while let Some(chunk) = field.chunk().await.map_err(|e| e.to_string())? {
                tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                    .await
                    .map_err(|e| format!("写入文件失败: {}", e))?;
                total_size += chunk.len() as u64;
            }

            tokio::io::AsyncWriteExt::flush(&mut file)
                .await
                .map_err(|e| format!("刷新文件缓冲区失败: {}", e))?;

            uploaded_files.push(UploadedFileInfo {
                file_path: file_path.to_string_lossy().to_string(),
                original_name: file_name,
                size: total_size,
            });
        }
    }

    Ok(Json(ApiResponse::success(UploadFilesResponse {
        uploaded_files,
    })))
}

// 批量上传并直接导入的处理器
async fn handler_upload_and_import_files(
    state: axum::extract::State<State>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<UploadAndImportResponse>>, ApiError> {
    let mut uploaded_files = Vec::new();
    let mut room_id = 0i64;
    let upload_dir = std::env::temp_dir().join("bsr_uploads");

    // 确保上传目录存在
    if !upload_dir.exists() {
        std::fs::create_dir_all(&upload_dir).map_err(|e| format!("创建上传目录失败: {}", e))?;
    }

    // 处理multipart表单数据
    while let Some(mut field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        if let Some(name) = field.name() {
            match name {
                "room_id" => {
                    // 读取房间ID
                    let text = field.text().await.map_err(|e| e.to_string())?;
                    room_id = text.parse().unwrap_or(0);
                }
                "files" => {
                    // 处理文件上传
                    if let Some(file_name) = field.file_name() {
                        let file_name = file_name.to_string();

                        // 检查文件格式
                        let extension = std::path::Path::new(&file_name)
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .unwrap_or("")
                            .to_lowercase();

                        let supported_extensions =
                            ["mp4", "mkv", "avi", "mov", "wmv", "flv", "m4v", "webm"];
                        if !supported_extensions.iter().any(|&ext| ext == extension) {
                            return Err(ApiError(format!(
                                "不支持的文件格式: {}。支持的格式: {}",
                                extension,
                                supported_extensions.join(", ")
                            )));
                        }

                        // 生成唯一的文件名
                        let timestamp = chrono::Utc::now().timestamp();
                        let sanitized_name = sanitize_filename_advanced(&file_name, None);
                        let unique_name = format!("{}_{}", timestamp, sanitized_name);
                        let file_path = upload_dir.join(&unique_name);

                        // 流式保存文件，避免大文件内存占用
                        let mut file = tokio::fs::File::create(&file_path)
                            .await
                            .map_err(|e| format!("创建文件失败: {}", e))?;

                        let mut total_size = 0u64;
                        while let Some(chunk) = field.chunk().await.map_err(|e| e.to_string())? {
                            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                                .await
                                .map_err(|e| format!("写入文件失败: {}", e))?;
                            total_size += chunk.len() as u64;
                        }

                        tokio::io::AsyncWriteExt::flush(&mut file)
                            .await
                            .map_err(|e| format!("刷新文件缓冲区失败: {}", e))?;

                        uploaded_files.push(UploadedFileInfo {
                            file_path: file_path.to_string_lossy().to_string(),
                            original_name: file_name,
                            size: total_size,
                        });
                    }
                }
                _ => {
                    // 忽略其他字段
                    let _ = field.bytes().await;
                }
            }
        }
    }

    if uploaded_files.is_empty() {
        return Err(ApiError("没有上传任何文件".to_string()));
    }

    // 生成批量导入的事件ID
    let event_id = format!("upload_import_{}", chrono::Utc::now().timestamp());

    // 启动批量导入任务
    let file_paths: Vec<String> = uploaded_files.iter().map(|f| f.file_path.clone()).collect();

    // 异步执行批量导入，不阻塞响应
    let state_clone = state.0.clone();
    let event_id_clone = event_id.clone();
    tokio::spawn(async move {
        if let Err(e) =
            batch_import_external_videos(state_clone, event_id_clone, file_paths, room_id).await
        {
            log::error!("批量导入上传文件失败: {}", e);
        }
    });

    Ok(Json(ApiResponse::success(UploadAndImportResponse {
        event_id,
        uploaded_files,
    })))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsoleLogRequest {
    level: String,
    message: String,
}

async fn handler_get_disk_info(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<DiskInfo>>, ApiError> {
    let disk_info = get_disk_info(state.0)
        .await
        .map_err(|_| "Failed to get disk info")?;
    Ok(Json(ApiResponse::success(disk_info)))
}

async fn handler_console_log(
    state: axum::extract::State<State>,
    Json(param): Json<ConsoleLogRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let _ = console_log(state.0, &param.level, &param.message).await;
    Ok(Json(ApiResponse::success(())))
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
) -> Result<impl IntoResponse, ApiError> {
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
        _ => return Err(ApiError("Unsupported HTTP method".to_string())),
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportDanmuRequest {
    options: ExportDanmuOptions,
}

async fn handler_export_danmu(
    state: axum::extract::State<State>,
    Json(params): Json<ExportDanmuRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let result = export_danmu(state.0, params.options).await?;
    Ok(Json(ApiResponse::success(result)))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteTaskRequest {
    id: String,
}

async fn handler_delete_task(
    state: axum::extract::State<State>,
    Json(params): Json<DeleteTaskRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    delete_task(state.0, &params.id).await?;
    Ok(Json(ApiResponse::success(())))
}

async fn handler_get_tasks(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<TaskRow>>>, ApiError> {
    let tasks = get_tasks(state.0).await?;
    Ok(Json(ApiResponse::success(tasks)))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenericFfmpegCommandRequest {
    args: Vec<String>,
}

async fn handler_generic_ffmpeg_command(
    state: axum::extract::State<State>,
    Json(params): Json<GenericFfmpegCommandRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    let result = generic_ffmpeg_command(state.0, params.args).await?;
    Ok(Json(ApiResponse::success(result)))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListFolderRequest {
    path: String,
}

async fn handler_list_folder(
    state: axum::extract::State<State>,
    Json(params): Json<ListFolderRequest>,
) -> Result<Json<ApiResponse<Vec<String>>>, ApiError> {
    let result = list_folder(state.0, params.path).await?;
    Ok(Json(ApiResponse::success(result)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileUploadResponse {
    file_path: String,
    file_name: String,
    file_size: u64,
}

async fn handler_upload_file(
    state: axum::extract::State<State>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<FileUploadResponse>>, ApiError> {
    if state.readonly {
        return Err(ApiError("Server is in readonly mode".to_string()));
    }

    let mut file_name = String::new();
    let mut uploaded_file_path: Option<PathBuf> = None;
    let mut file_size = 0u64;
    let mut _room_id = 0i64;

    while let Some(mut field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                file_name = field.file_name().unwrap_or("unknown").to_string();

                // 创建上传目录
                let config = state.config.read().await;
                let upload_dir = std::path::Path::new(&config.cache).join("uploads");
                if !upload_dir.exists() {
                    std::fs::create_dir_all(&upload_dir).map_err(|e| e.to_string())?;
                }

                // 生成唯一文件名避免冲突
                let timestamp = chrono::Utc::now().timestamp();
                let extension = std::path::Path::new(&file_name)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("");
                let base_name = std::path::Path::new(&file_name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("upload");

                let unique_filename = if extension.is_empty() {
                    format!("{}_{}", base_name, timestamp)
                } else {
                    format!("{}_{}.{}", base_name, timestamp, extension)
                };

                let file_path = upload_dir.join(&unique_filename);

                // 流式保存文件，避免大文件内存占用
                let mut file = tokio::fs::File::create(&file_path)
                    .await
                    .map_err(|e| format!("创建文件失败: {}", e))?;

                while let Some(chunk) = field.chunk().await.map_err(|e| e.to_string())? {
                    tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                        .await
                        .map_err(|e| format!("写入文件失败: {}", e))?;
                    file_size += chunk.len() as u64;
                }

                tokio::io::AsyncWriteExt::flush(&mut file)
                    .await
                    .map_err(|e| format!("刷新文件缓冲区失败: {}", e))?;

                uploaded_file_path = Some(file_path);
                file_name = unique_filename;
            }
            "roomId" => {
                let room_id_str = field.text().await.map_err(|e| e.to_string())?;
                _room_id = room_id_str.parse().unwrap_or(0);
            }
            _ => {}
        }
    }

    if file_name.is_empty() || uploaded_file_path.is_none() {
        return Err(ApiError("No file uploaded".to_string()));
    }

    let file_path = uploaded_file_path.unwrap();
    let file_path_str = file_path.to_string_lossy().to_string();

    log::info!("File uploaded: {} ({} bytes)", file_path_str, file_size);

    Ok(Json(ApiResponse::success(FileUploadResponse {
        file_path: file_path_str,
        file_name,
        file_size,
    })))
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
    let content_type = match filename.split('.').next_back() {
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

const MAX_BODY_SIZE: usize = 10 * 1024 * 1024 * 1024;

pub async fn start_api_server(state: State) {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let mut app = Router::new()
        // Serve static files from dist directory
        .nest_service("/", ServeDir::new("./dist"))
        // Account commands
        .route("/api/get_accounts", post(handler_get_accounts))
        .route("/api/get_account_count", post(handler_get_account_count));

    // Only add add/remove routes if not in readonly mode
    if !state.readonly {
        app = app
            .route("/api/get_qr", post(handler_get_qr))
            .route("/api/get_qr_status", post(handler_get_qr_status))
            .route("/api/add_account", post(handler_add_account))
            .route("/api/remove_account", post(handler_remove_account))
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
            .route("/api/add_recorder", post(handler_add_recorder))
            .route("/api/remove_recorder", post(handler_remove_recorder))
            .route("/api/delete_archive", post(handler_delete_archive))
            .route("/api/delete_archives", post(handler_delete_archives))
            .route("/api/send_danmaku", post(handler_send_danmaku))
            .route("/api/set_enable", post(handler_set_enable))
            .route("/api/upload_procedure", post(handler_upload_procedure))
            .route("/api/cancel", post(handler_cancel))
            .route("/api/delete_video", post(handler_delete_video))
            .route(
                "/api/generate_video_subtitle",
                post(handler_generate_video_subtitle),
            )
            .route(
                "/api/generate_archive_subtitle",
                post(handler_generate_archive_subtitle),
            )
            .route(
                "/api/generic_ffmpeg_command",
                post(handler_generic_ffmpeg_command),
            )
            .route(
                "/api/update_video_subtitle",
                post(handler_update_video_subtitle),
            )
            .route("/api/update_video_cover", post(handler_update_video_cover))
            .route("/api/update_video_note", post(handler_update_video_note))
            .route(
                "/api/encode_video_subtitle",
                post(handler_encode_video_subtitle),
            )
            .route(
                "/api/import_external_video",
                post(handler_import_external_video),
            )
            .route("/api/clip_video", post(handler_clip_video))
            .route(
                "/api/generate_whole_clip",
                post(handler_generate_whole_clip),
            )
            .route("/api/update_notify", post(handler_update_notify))
            .route(
                "/api/update_status_check_interval",
                post(handler_update_status_check_interval),
            )
            .route(
                "/api/update_whisper_prompt",
                post(handler_update_whisper_prompt),
            )
            .route(
                "/api/update_subtitle_generator_type",
                post(handler_update_subtitle_generator_type),
            )
            .route(
                "/api/update_openai_api_endpoint",
                post(handler_update_openai_api_endpoint),
            )
            .route(
                "/api/update_openai_api_key",
                post(handler_update_openai_api_key),
            )
            .route(
                "/api/update_auto_generate",
                post(handler_update_auto_generate),
            )
            .route(
                "/api/update_whisper_language",
                post(handler_update_whisper_language),
            )
            .route("/api/update_webhook_url", post(handler_update_webhook_url))
            .route(
                "/api/batch_import_external_videos",
                post(handler_batch_import_external_videos),
            )
            .route(
                "/api/get_import_progress",
                post(handler_get_import_progress),
            )
            .route("/api/upload_files", post(handler_upload_files))
            .route(
                "/api/upload_and_import_files",
                post(handler_upload_and_import_files),
            );
    } else {
        log::info!("Running in readonly mode, some api routes are disabled");
    }

    let cache_path = state.config.read().await.cache.clone();
    let output_path = state.config.read().await.output.clone();

    app = app
        // Config commands
        .route("/api/get_config", post(handler_get_config))
        // Message commands
        .route("/api/get_messages", post(handler_get_messages))
        .route("/api/read_message", post(handler_read_message))
        .route("/api/delete_message", post(handler_delete_message))
        // Recorder commands
        .route("/api/get_recorder_list", post(handler_get_recorder_list))
        .route("/api/get_room_info", post(handler_get_room_info))
        .route("/api/get_archives", post(handler_get_archives))
        .route("/api/get_archive", post(handler_get_archive))
        .route(
            "/api/get_archives_by_parent_id",
            post(handler_get_archives_by_parent_id),
        )
        .route(
            "/api/get_archive_disk_usage",
            post(handler_get_archive_disk_usage),
        )
        .route(
            "/api/get_archive_subtitle",
            post(handler_get_archive_subtitle),
        )
        .route("/api/get_danmu_record", post(handler_get_danmu_record))
        .route("/api/get_total_length", post(handler_get_total_length))
        .route(
            "/api/get_today_record_count",
            post(handler_get_today_record_count),
        )
        .route("/api/get_recent_record", post(handler_get_recent_record))
        // Video commands
        .route("/api/clip_range", post(handler_clip_range))
        .route("/api/get_video", post(handler_get_video))
        .route("/api/get_videos", post(handler_get_videos))
        .route("/api/get_video_cover", post(handler_get_video_cover))
        .route("/api/get_all_videos", post(handler_get_all_videos))
        .route("/api/get_video_typelist", post(handler_get_video_typelist))
        .route("/api/get_video_subtitle", post(handler_get_video_subtitle))
        .route("/api/get_file_size", post(handler_get_file_size))
        .route("/api/delete_task", post(handler_delete_task))
        .route("/api/get_tasks", post(handler_get_tasks))
        .route("/api/export_danmu", post(handler_export_danmu))
        // Utils commands
        .route("/api/get_disk_info", post(handler_get_disk_info))
        .route("/api/console_log", post(handler_console_log))
        .route("/api/list_folder", post(handler_list_folder))
        .route("/api/fetch", post(handler_fetch))
        .route("/api/upload_file", post(handler_upload_file))
        .route("/api/image/:video_id", get(handler_image_base64))
        .route("/hls/*uri", get(handler_hls))
        .nest_service("/output", ServeDir::new(output_path))
        .nest_service("/cache", ServeDir::new(cache_path));

    let websocket_layer = websocket::create_websocket_server(state.clone()).await;

    let router = app
        .layer(websocket_layer)
        .layer(cors)
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(state);

    let addr = "0.0.0.0:3000";
    log::info!("Starting API server on http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            log::info!("API server listening on http://{}", addr);
            listener
        }
        Err(e) => {
            log::error!("Failed to bind to address {}: {}", addr, e);
            log::error!("Please check if the port is already in use or try a different port");
            return;
        }
    };

    if let Err(e) = axum::serve(listener, router).await {
        log::error!("Server error: {}", e);
    }
}
