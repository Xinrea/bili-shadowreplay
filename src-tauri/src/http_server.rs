use std::fmt::{self, Display};

use crate::{
    config::Config,
    database::{
        account::AccountRow,
        message::MessageRow,
        record::RecordRow,
        recorder::RecorderRow,
        task::TaskRow,
        video::{VideoNoCover, VideoRow},
    },
    handlers::{
        account::{
            add_account, get_account_count, get_accounts, get_qr, get_qr_status, remove_account,
        },
        config::{
            get_config, update_auto_generate, update_clip_name_format, update_notify,
            update_openai_api_endpoint, update_openai_api_key, update_status_check_interval,
            update_subtitle_generator_type, update_subtitle_setting, update_user_agent,
            update_whisper_language, update_whisper_model, update_whisper_prompt,
        },
        message::{delete_message, get_messages, read_message},
        recorder::{
            add_recorder, delete_archive, export_danmu, fetch_hls, generate_archive_subtitle,
            get_archive, get_archive_subtitle, get_archives, get_danmu_record, get_recent_record,
            get_recorder_list, get_room_info, get_today_record_count, get_total_length,
            remove_recorder, send_danmaku, set_enable, ExportDanmuOptions,
        },
        task::{delete_task, get_tasks},
        utils::{console_log, get_disk_info, list_folder, DiskInfo},
        video::{
            cancel, clip_range, clip_video, delete_video, encode_video_subtitle, generate_video_subtitle,
            generic_ffmpeg_command, get_all_videos, get_file_size, get_video, get_video_cover, get_video_subtitle,
            get_video_typelist, get_videos, import_external_video, update_video_cover, update_video_subtitle,
            upload_procedure,
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
use axum::{extract::Query, response::sse};
use axum::{
    extract::{DefaultBodyLimit, Json, Path},
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
    uid: u64,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateUserAgentRequest {
    user_agent: String,
}

async fn handler_update_user_agent(
    state: axum::extract::State<State>,
    Json(user_agent): Json<UpdateUserAgentRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    update_user_agent(state.0, user_agent.user_agent)
        .await
        .expect("Failed to update user agent");
    Ok(Json(ApiResponse::success(())))
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
    room_id: u64,
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
    room_id: u64,
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
    room_id: u64,
}

async fn handler_get_room_info(
    state: axum::extract::State<State>,
    Json(param): Json<GetRoomInfoRequest>,
) -> Result<Json<ApiResponse<RecorderInfo>>, ApiError> {
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
) -> Result<Json<ApiResponse<Vec<RecordRow>>>, ApiError> {
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
) -> Result<Json<ApiResponse<RecordRow>>, ApiError> {
    let archive = get_archive(state.0, param.room_id, param.live_id).await?;
    Ok(Json(ApiResponse::success(archive)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArchiveSubtitleRequest {
    platform: String,
    room_id: u64,
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
    room_id: u64,
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
    room_id: u64,
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
struct GetDanmuRecordRequest {
    platform: String,
    room_id: u64,
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
    uid: u64,
    room_id: u64,
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
    room_id: u64,
    offset: u64,
    limit: u64,
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
    room_id: u64,
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
    uid: u64,
    room_id: u64,
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
    room_id: u64,
}

async fn handler_get_videos(
    state: axum::extract::State<State>,
    Json(param): Json<GetVideosRequest>,
) -> Result<Json<ApiResponse<Vec<VideoNoCover>>>, ApiError> {
    let videos = get_videos(state.0, param.room_id).await?;
    Ok(Json(ApiResponse::success(videos)))
}

async fn handler_get_all_videos(
    state: axum::extract::State<State>,
) -> Result<Json<ApiResponse<Vec<VideoNoCover>>>, ApiError> {
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
    room_id: u64,
}

async fn handler_import_external_video(
    state: axum::extract::State<State>,
    Json(param): Json<ImportExternalVideoRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    import_external_video(state.0, param.event_id.clone(), param.file_path, param.room_id).await?;
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
    ).await?;
    Ok(Json(ApiResponse::success(param.event_id)))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFileSizeRequest {
    file_path: String,
}

async fn handler_get_file_size(
    state: axum::extract::State<State>,
    Json(param): Json<GetFileSizeRequest>,
) -> Result<Json<ApiResponse<u64>>, ApiError> {
    let file_size = get_file_size(param.file_path).await?;
    Ok(Json(ApiResponse::success(file_size)))
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

        // Only set Content-Disposition for non-video files to allow inline playback
        if !matches!(content_type, "video/mp4" | "video/webm" | "video/x-m4v" | "video/x-matroska" | "video/x-msvideo") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            headers.insert(
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename)
                    .parse()
                    .unwrap(),
            );
        }

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
                    Event::ProgressUpdate { id, content } => {
                        sse::Event::default().event("progress-update").data(format!(
                            r#"{{"id":"{}","content":"{}"}}"#,
                            id,
                            content.replace('\n', "\\n").replace('\r', "\\r")
                        ))
                    }
                    Event::ProgressFinished {
                        id,
                        success,
                        message,
                    } => sse::Event::default()
                        .event("progress-finished")
                        .data(format!(
                            r#"{{"id":"{}","success":{},"message":"{}"}}"#,
                            id,
                            success,
                            message.replace('\n', "\\n").replace('\r', "\\r")
                        )),
                    Event::DanmuReceived { room, ts, content } => sse::Event::default()
                        .event(format!("danmu:{}", room))
                        .data(format!(
                            r#"{{"ts":"{}","content":"{}"}}"#,
                            ts,
                            content.replace('\n', "\\n").replace('\r', "\\r")
                        )),
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
            .route(
                "/api/encode_video_subtitle",
                post(handler_encode_video_subtitle),
            )
            .route(
                "/api/import_external_video",
                post(handler_import_external_video),
            )
            .route("/api/clip_video", post(handler_clip_video))
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
            .route("/api/update_user_agent", post(handler_update_user_agent));
    } else {
        log::info!("Running in readonly mode, some api routes are disabled");
    }

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
        .route("/hls/*uri", get(handler_hls))
        .route("/output/*uri", get(handler_output))
        .route("/api/sse", get(handler_sse));

    let router = app
        .layer(cors)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .with_state(state);

    let addr = "0.0.0.0:3000";
    log::info!("API server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
