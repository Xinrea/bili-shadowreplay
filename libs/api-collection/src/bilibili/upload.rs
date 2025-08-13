pub mod response;

use std::path::Path;

use crate::errors::ApiCollectionError;
use reqwest::Client;
use std::time::Duration;
use tokio::io::AsyncReadExt;

async fn preupload_video(
    client: &Client,
    video_file: &Path,
) -> Result<response::PreuploadResponse, ApiCollectionError> {
    let url = format!(
        "https://member.bilibili.com/preupload?name={}&r=upos&profile=ugcfx/bup",
        video_file.file_name().unwrap().to_str().unwrap()
    );
    let response = client
        .get(&url)
        .send()
        .await?
        .json::<response::PreuploadResponse>()
        .await?;
    Ok(response)
}

async fn post_video_meta(
    client: &Client,
    preupload_response: &response::PreuploadResponse,
    video_file: &Path,
) -> Result<response::PostVideoMetaResponse, ApiCollectionError> {
    let url = format!(
        "https:{}{}?uploads=&output=json&profile=ugcfx/bup&filesize={}&partsize={}&biz_id={}",
        preupload_response.endpoint,
        preupload_response.upos_uri.replace("upos:/", ""),
        video_file.metadata().unwrap().len(),
        preupload_response.chunk_size,
        preupload_response.biz_id
    );
    let response = client
        .post(&url)
        .header("X-Upos-Auth", &preupload_response.auth)
        .send()
        .await?
        .json::<response::PostVideoMetaResponse>()
        .await?;
    Ok(response)
}

#[derive(Clone)]
struct UploadParams<'a> {
    preupload_response: &'a response::PreuploadResponse,
    post_video_meta_response: &'a response::PostVideoMetaResponse,
    video_file: &'a Path,
}

async fn upload_video(
    client: &Client,
    params: UploadParams<'_>,
) -> Result<usize, ApiCollectionError> {
    let mut file = tokio::fs::File::open(params.video_file).await?;
    let mut buffer = vec![0; params.preupload_response.chunk_size];
    let file_size = params.video_file.metadata()?.len();
    let chunk_size = params.preupload_response.chunk_size as u64;
    let total_chunks = (file_size as f64 / chunk_size as f64).ceil() as usize;

    let mut chunk = 0;
    let mut read_total = 0;
    let max_retries = 3;
    let timeout = Duration::from_secs(30);

    while let Ok(size) = file.read(&mut buffer[read_total..]).await {
        read_total += size;
        if size > 0 && (read_total as u64) < chunk_size {
            continue;
        }
        if size == 0 && read_total == 0 {
            break;
        }

        let mut retry_count = 0;
        let mut success = false;

        while retry_count < max_retries && !success {
            let url = format!(
                "https:{}{}?partNumber={}&uploadId={}&chunk={}&chunks={}&size={}&start={}&end={}&total={}",
                params.preupload_response.endpoint,
                params.preupload_response.upos_uri.replace("upos:/", ""),
                chunk + 1,
                params.post_video_meta_response.upload_id,
                chunk,
                total_chunks,
                read_total,
                chunk * params.preupload_response.chunk_size,
                chunk * params.preupload_response.chunk_size + read_total,
                params.video_file.metadata().unwrap().len()
            );

            match client
                .put(&url)
                .header("X-Upos-Auth", &params.preupload_response.auth)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", read_total.to_string())
                .timeout(timeout)
                .body(buffer[..read_total].to_vec())
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        success = true;
                        let _ = response.text().await?;
                    } else {
                        retry_count += 1;
                        if retry_count < max_retries {
                            tokio::time::sleep(Duration::from_secs(2u64.pow(retry_count as u32)))
                                .await;
                        }
                    }
                }
                Err(_) => {
                    retry_count += 1;
                    if retry_count < max_retries {
                        tokio::time::sleep(Duration::from_secs(2u64.pow(retry_count as u32))).await;
                    }
                }
            }
        }

        if !success {
            return Err(ApiCollectionError::UploadError {
                err: format!(
                    "Failed to upload chunk {} after {} retries",
                    chunk, max_retries
                ),
            });
        }

        chunk += 1;
        read_total = 0;
    }
    Ok(total_chunks)
}

async fn end_upload(
    client: &Client,
    preupload_response: &response::PreuploadResponse,
    post_video_meta_response: &response::PostVideoMetaResponse,
    chunks: usize,
) -> Result<(), ApiCollectionError> {
    let url = format!(
        "https:{}{}?output=json&name={}&profile=ugcfx/bup&uploadId={}&biz_id={}",
        preupload_response.endpoint,
        preupload_response.upos_uri.replace("upos:/", ""),
        preupload_response.upos_uri,
        post_video_meta_response.upload_id,
        preupload_response.biz_id
    );
    let parts: Vec<serde_json::Value> = (1..=chunks)
        .map(|i| serde_json::json!({ "partNumber": i, "eTag": "etag" }))
        .collect();
    let body = serde_json::json!({ "parts": parts });
    client
        .post(&url)
        .header("X-Upos-Auth", &preupload_response.auth)
        .header("Content-Type", "application/json; charset=UTF-8")
        .body(body.to_string())
        .send()
        .await?
        .text()
        .await?;
    Ok(())
}

pub async fn prepare_video(
    client: &Client,
    video_file: &Path,
) -> Result<response::Video, ApiCollectionError> {
    let preupload = preupload_video(client, video_file).await?;
    let metaposted = post_video_meta(client, &preupload, video_file).await?;
    let uploaded = upload_video(
        client,
        UploadParams {
            preupload_response: &preupload,
            post_video_meta_response: &metaposted,
            video_file,
        },
    )
    .await?;
    end_upload(client, &preupload, &metaposted, uploaded).await?;
    let filename = Path::new(&metaposted.key)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap();
    Ok(response::Video {
        title: "".to_string(),
        filename: filename.to_string(),
        desc: "".to_string(),
        cid: preupload.biz_id,
    })
}

pub async fn submit_video(
    client: &Client,
    csrf: &str,
    profile_template: &response::Profile,
    video: &response::Video,
) -> Result<response::VideoSubmitData, ApiCollectionError> {
    let url = format!(
        "https://member.bilibili.com/x/vu/web/add/v3?ts={}&csrf={}",
        chrono::Local::now().timestamp(),
        csrf
    );
    let mut preprofile = profile_template.clone();
    preprofile.videos.push(video.clone());
    match client
        .post(&url)
        .header("Content-Type", "application/json; charset=UTF-8")
        .body(serde_json::ser::to_string(&preprofile).unwrap_or("".to_string()))
        .send()
        .await
    {
        Ok(raw_resp) => {
            let json: serde_json::Value = raw_resp.json().await?;
            if let Ok(resp) = serde_json::from_value::<response::GeneralResponse>(json.clone()) {
                match resp.data {
                    response::Data::VideoSubmit(data) => Ok(data),
                    _ => Err(ApiCollectionError::InvalidValue {
                        key: "data.video_submit".to_string(),
                        value: json.to_string(),
                    }),
                }
            } else {
                Err(ApiCollectionError::InvalidValue {
                    key: "data.video_submit".to_string(),
                    value: json.to_string(),
                })
            }
        }
        Err(e) => Err(ApiCollectionError::RequestError { err: e.to_string() }),
    }
}

pub async fn upload_cover(
    client: &Client,
    csrf: &str,
    cover: &str,
) -> Result<String, ApiCollectionError> {
    let url = format!(
        "https://member.bilibili.com/x/vu/web/cover/up?ts={}",
        chrono::Local::now().timestamp(),
    );
    let params = [("csrf", csrf.to_string()), ("cover", cover.to_string())];
    match client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await
    {
        Ok(raw_resp) => {
            let json: serde_json::Value = raw_resp.json().await?;
            if let Ok(resp) = serde_json::from_value::<response::GeneralResponse>(json.clone()) {
                match resp.data {
                    response::Data::Cover(data) => Ok(data.url),
                    _ => Err(ApiCollectionError::InvalidValue {
                        key: "data.cover".to_string(),
                        value: json.to_string(),
                    }),
                }
            } else {
                Err(ApiCollectionError::InvalidValue {
                    key: "data.code".to_string(),
                    value: json.to_string(),
                })
            }
        }
        Err(e) => Err(ApiCollectionError::RequestError { err: e.to_string() }),
    }
}

pub async fn send_danmaku(
    client: &Client,
    csrf: &str,
    room_id: u64,
    message: &str,
) -> Result<(), ApiCollectionError> {
    let url = "https://api.live.bilibili.com/msg/send".to_string();
    let params = [
        ("bubble", "0"),
        ("msg", message),
        ("color", "16777215"),
        ("mode", "1"),
        ("fontsize", "25"),
        ("room_type", "0"),
        ("rnd", &format!("{}", chrono::Local::now().timestamp())),
        ("roomid", &format!("{}", room_id)),
        ("csrf", csrf),
        ("csrf_token", csrf),
    ];
    let _ = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    Ok(())
}

pub async fn get_video_typelist(
    client: &Client,
) -> Result<Vec<response::Typelist>, ApiCollectionError> {
    let url = "https://member.bilibili.com/x/vupre/web/archive/pre?lang=cn";
    let resp: response::GeneralResponse = client
        .get(url)
        .send()
        .await?
        .json::<response::GeneralResponse>()
        .await?;
    if resp.code == 0 {
        if let response::Data::VideoTypeList(data) = resp.data {
            Ok(data.typelist)
        } else {
            Err(ApiCollectionError::InvalidValue {
                key: "data.typelist".to_string(),
                value: serde_json::to_string(&resp.data).unwrap_or("".to_string()),
            })
        }
    } else {
        Err(ApiCollectionError::InvalidValue {
            key: "data.code".to_string(),
            value: serde_json::to_string(&resp.data).unwrap_or("".to_string()),
        })
    }
}
