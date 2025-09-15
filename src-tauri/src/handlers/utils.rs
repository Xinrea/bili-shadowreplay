use std::path::PathBuf;

use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use {
    crate::recorder::PlatformType,
    std::process::Command,
    tauri::State as TauriState,
    tauri::{Manager, Theme},
    tauri_utils::config::WindowEffectsConfig,
    tokio::fs::OpenOptions,
    tokio::io::AsyncWriteExt,
};

#[allow(dead_code)]
pub fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .args(["/select,", &path]) // The comma after select is not a typo
            .spawn()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::metadata;
        use std::path::PathBuf;
        if path.contains(",") {
            // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
            let new_path = match metadata(&path).unwrap().is_dir() {
                true => path,
                false => {
                    let mut path2 = PathBuf::from(path);
                    path2.pop();
                    path2.into_os_string().into_string().unwrap()
                }
            };
            Command::new("xdg-open")
                .arg(&new_path)
                .spawn()
                .unwrap()
                .await;
        } else {
            Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    format!("array:string:\"file://{path}\"").as_str(),
                    "string:\"\"",
                ])
                .spawn()
                .unwrap()
                .await;
        }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path])
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
    }
}

#[derive(serde::Serialize)]
pub struct DiskInfo {
    disk: String,
    total: u64,
    pub free: u64,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_disk_info(state: state_type!()) -> Result<DiskInfo, ()> {
    let cache = state.config.read().await.cache.clone();
    // if cache is relative path, convert it to absolute path
    let mut cache = PathBuf::from(&cache);
    if cache.is_relative() {
        // get current working directory
        let cwd = std::env::current_dir().unwrap();
        cache = cwd.join(cache);
    }

    get_disk_info_inner(cache).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn console_log(_state: state_type!(), level: &str, message: &str) -> Result<(), ()> {
    match level {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        "info" => log::info!("[frontend] {message}"),
        _ => log::debug!("[frontend] {message}"),
    }
    Ok(())
}

pub async fn get_disk_info_inner(target: PathBuf) -> Result<DiskInfo, ()> {
    #[cfg(target_os = "linux")]
    {
        // get disk info from df command
        let output = tokio::process::Command::new("df")
            .arg(target)
            .output()
            .await
            .unwrap();
        let output_str = String::from_utf8(output.stdout).unwrap();
        // Filesystem     1K-blocks     Used Available Use% Mounted on
        // /dev/nvme0n1p2 959218776 43826092 866593352   5% /app/cache
        let lines = output_str.lines().collect::<Vec<&str>>();
        if lines.len() < 2 {
            log::error!("df command output is too short: {}", output_str);
            return Err(());
        }
        let parts = lines[1].split_whitespace().collect::<Vec<&str>>();
        let disk = parts[0].to_string();
        let total = parts[1].parse::<u64>().unwrap() * 1024;
        let free = parts[3].parse::<u64>().unwrap() * 1024;

        Ok(DiskInfo { disk, total, free })
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        // check system disk info
        let disks = sysinfo::Disks::new_with_refreshed_list();
        // get target disk info
        let mut disk_info = DiskInfo {
            disk: String::new(),
            total: 0,
            free: 0,
        };

        // Find the disk with the longest matching mount point
        let mut longest_match = 0;
        for disk in disks.list() {
            let mount_point = disk.mount_point().to_str().unwrap();
            if target.starts_with(mount_point) && mount_point.split('/').count() > longest_match {
                disk_info.disk = mount_point.into();
                disk_info.total = disk.total_space();
                disk_info.free = disk.available_space();
                longest_match = mount_point.split('/').count();
            }
        }

        Ok(disk_info)
    }
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn export_to_file(
    _state: state_type!(),
    file_name: &str,
    content: &str,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_name)
        .await;
    if file.is_err() {
        return Err(format!("Open file failed: {}", file.err().unwrap()));
    }
    let mut file = file.unwrap();
    if let Err(e) = file.write_all(content.as_bytes()).await {
        return Err(format!("Write file failed: {e}"));
    }
    if let Err(e) = file.flush().await {
        return Err(format!("Flush file failed: {e}"));
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_log_folder(state: state_type!()) -> Result<(), String> {
    #[cfg(feature = "gui")]
    {
        let log_dir = state.app_handle.path().app_log_dir().unwrap();
        show_in_folder(log_dir.to_str().unwrap().to_string());
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_live(
    state: state_type!(),
    platform: String,
    room_id: i64,
    live_id: String,
) -> Result<(), String> {
    log::info!("Open player window: {room_id} {live_id}");
    #[cfg(feature = "gui")]
    {
        let platform = PlatformType::from_str(&platform).unwrap();
        let recorder_info = state
            .recorder_manager
            .get_recorder_info(platform, room_id)
            .await
            .unwrap();
        let builder = tauri::WebviewWindowBuilder::new(
            &state.app_handle,
            format!("Live:{room_id}:{live_id}"),
            tauri::WebviewUrl::App(
                format!(
                    "index_live.html?platform={}&room_id={}&live_id={}",
                    platform.as_str(),
                    room_id,
                    live_id
                )
                .into(),
            ),
        )
        .title(format!(
            "Live[{}] {}",
            room_id, recorder_info.room_info.room_title
        ))
        .theme(Some(Theme::Light))
        .inner_size(1200.0, 800.0)
        .effects(WindowEffectsConfig {
            effects: vec![
                tauri_utils::WindowEffect::Tabbed,
                tauri_utils::WindowEffect::Mica,
            ],
            state: None,
            radius: None,
            color: None,
        });

        if let Err(e) = builder.decorations(true).build() {
            log::error!("live window build failed: {e}");
        }
    }

    Ok(())
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_clip(state: state_type!(), video_id: i64) -> Result<(), String> {
    log::info!("Open clip window: {video_id}");
    let builder = tauri::WebviewWindowBuilder::new(
        &state.app_handle,
        format!("Clip:{video_id}"),
        tauri::WebviewUrl::App(format!("index_clip.html?id={video_id}").into()),
    )
    .title(format!("Clip window:{video_id}"))
    .theme(Some(Theme::Light))
    .inner_size(1200.0, 800.0)
    .effects(WindowEffectsConfig {
        effects: vec![
            tauri_utils::WindowEffect::Tabbed,
            tauri_utils::WindowEffect::Mica,
        ],
        state: None,
        radius: None,
        color: None,
    });

    if let Err(e) = builder.decorations(true).build() {
        log::error!("clip window build failed: {e}");
    }

    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn list_folder(_state: state_type!(), path: String) -> Result<Vec<String>, String> {
    let path = PathBuf::from(path);
    let entries = std::fs::read_dir(path);
    if entries.is_err() {
        return Err(format!("Read directory failed: {}", entries.err().unwrap()));
    }
    let mut files = Vec::new();
    for entry in entries.unwrap().flatten() {
        files.push(entry.path().to_str().unwrap().to_string());
    }
    Ok(files)
}

/// 高级文件名清理函数，全面处理各种危险字符和控制字符
///
/// 适用于需要严格文件名清理的场景，支持中文字符
///
/// # 参数
/// - `name`: 需要清理的文件名
/// - `max_length`: 最大长度限制（默认100字符）
///
/// # 返回
/// 经过全面清理的安全文件名
#[cfg(feature = "headless")]
pub fn sanitize_filename_advanced(name: &str, max_length: Option<usize>) -> String {
    let max_len = max_length.unwrap_or(100);

    // 先清理所有字符
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            // 文件系统危险字符
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            // 控制字符和不可见字符
            c if c.is_control() => '_',
            // 保留安全的字符（白名单）
            c if c.is_alphanumeric()
                || c == ' '
                || c == '.'
                || c == '-'
                || c == '_'
                || c == '('
                || c == ')'
                || c == '['
                || c == ']'
                || c == '《'
                || c == '》'
                || c == '（'
                || c == '）' =>
            {
                c
            }
            // 其他字符替换为下划线
            _ => '_',
        })
        .collect();

    // 如果清理后的长度在限制内，直接返回
    if cleaned.chars().count() <= max_len {
        return cleaned;
    }

    // 智能截断：保护文件扩展名
    if let Some(dot_pos) = cleaned.rfind('.') {
        let extension = &cleaned[dot_pos..];
        let main_part = &cleaned[..dot_pos];

        // 确保扩展名不会太长（最多10个字符，包括点号）
        if extension.chars().count() <= 10 {
            let ext_len = extension.chars().count();
            let available_for_main = max_len.saturating_sub(ext_len);

            if available_for_main > 0 {
                let truncated_main: String = main_part.chars().take(available_for_main).collect();
                return format!("{}{}", truncated_main, extension);
            }
        }
    }

    // 如果没有扩展名或扩展名太长，直接截断
    cleaned.chars().take(max_len).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "headless")]
    fn test_sanitize_filename_advanced() {
        use super::sanitize_filename_advanced;

        assert_eq!(
            sanitize_filename_advanced("test<>file.txt", None),
            "test__file.txt"
        );
        assert_eq!(sanitize_filename_advanced("文件名.txt", None), "文件名.txt");
        assert_eq!(
            sanitize_filename_advanced("《视频》（高清）.mp4", None),
            "《视频》（高清）.mp4"
        );
        assert_eq!(
            sanitize_filename_advanced("file\x00with\x01control.txt", None),
            "file_with_control.txt"
        );

        // 测试空白字符处理（函数不自动移除空白字符）
        assert_eq!(
            sanitize_filename_advanced("   .hidden_file.txt   ", None),
            "   .hidden_file.txt   "
        );
        assert_eq!(
            sanitize_filename_advanced("  normal_file.mp4  ", None),
            "  normal_file.mp4  "
        );

        // 测试特殊字符替换
        assert_eq!(
            sanitize_filename_advanced("file@#$%^&.txt", None),
            "file______.txt"
        );

        // 测试长度限制 - 无扩展名
        let long_name = "测试".repeat(60);
        let result = sanitize_filename_advanced(&long_name, Some(10));
        assert_eq!(result.chars().count(), 10);

        // 测试长度限制 - 有扩展名
        let long_name_with_ext = format!("{}.txt", "测试".repeat(60));
        let result = sanitize_filename_advanced(&long_name_with_ext, Some(10));
        assert!(result.ends_with(".txt"));
        assert_eq!(result.chars().count(), 10); // 6个测试字符 + .txt (4个字符)

        // 测试短文件名不被截断
        let short_name = "test.mp4";
        let result = sanitize_filename_advanced(short_name, Some(50));
        assert_eq!(result, "test.mp4");
    }
}
