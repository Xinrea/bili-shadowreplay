use std::path::PathBuf;

use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use {
    recorder::platforms::PlatformType, std::process::Command, tauri::Manager,
    tauri::State as TauriState, tauri_utils::config::WindowEffectsConfig, tokio::fs::OpenOptions,
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
pub fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        // The comma after select is not a typo
        if let Err(e) = Command::new("explorer").args(["/select,", &path]).spawn() {
            log::error!("Failed to open folder {path}: {e}");
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::metadata;
        use std::path::PathBuf;
        if path.contains(",") {
            // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
            let is_dir = metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
            let new_path = if is_dir {
                path
            } else {
                let mut path2 = PathBuf::from(path);
                path2.pop();
                path2.to_string_lossy().into_owned()
            };
            if let Err(e) = Command::new("xdg-open").arg(&new_path).spawn() {
                log::error!("Failed to open folder {new_path}: {e}");
            }
        } else if let Err(e) = Command::new("dbus-send")
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
        {
            log::error!("Failed to reveal {path}: {e}");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = Command::new("open").args(["-R", &path]).spawn() {
            log::error!("Failed to reveal {path}: {e}");
        }
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
        let cwd = std::env::current_dir()
            .map_err(|e| log::error!("Failed to get current directory: {e}"))?;
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
            .map_err(|e| log::error!("Failed to run df: {e}"))?;
        let output_str =
            String::from_utf8(output.stdout).map_err(|e| log::error!("Invalid df output: {e}"))?;
        // Filesystem     1K-blocks     Used Available Use% Mounted on
        // /dev/nvme0n1p2 959218776 43826092 866593352   5% /app/cache
        let lines = output_str.lines().collect::<Vec<&str>>();
        if lines.len() < 2 {
            log::error!("df command output is too short: {}", output_str);
            return Err(());
        }
        let parts = lines[1].split_whitespace().collect::<Vec<&str>>();
        if parts.len() < 4 {
            log::error!("Unexpected df output: {}", lines[1]);
            return Err(());
        }
        let disk = parts[0].to_string();
        let total = parts[1]
            .parse::<u64>()
            .map_err(|e| log::error!("Failed to parse total blocks: {e}"))?
            * 1024;
        let free = parts[3]
            .parse::<u64>()
            .map_err(|e| log::error!("Failed to parse free blocks: {e}"))?
            * 1024;

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
            let mount_point = disk.mount_point().to_string_lossy();
            if target.starts_with(&*mount_point) && mount_point.split('/').count() > longest_match {
                disk_info.disk = mount_point.to_string();
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
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_name)
        .await
        .map_err(|e| format!("Open file failed: {e}"))?;
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
        let log_dir = state
            .app_handle
            .path()
            .app_log_dir()
            .map_err(|e| format!("Failed to get log dir: {e}"))?;
        show_in_folder(log_dir.to_string_lossy().into_owned());
    }
    Ok(())
}

/// Preview windows draw their own title bar (`TitleBar.svelte`): on macOS it
/// sits under the native traffic lights, elsewhere it replaces the decorations.
/// Keep the traffic light position in sync with `tauri.macos.conf.json`.
#[cfg(feature = "gui")]
fn with_custom_titlebar<'a, R: tauri::Runtime, M: Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "macos")]
    {
        builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(16.0, 22.0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder.decorations(false)
    }
}

/// Changes the calling window's title. On macOS, AppKit lays the title bar out
/// again on every title change, which moves the traffic lights back to their
/// default position. Marking the content view dirty in the same main-thread
/// turn makes wry re-apply the custom position when it redraws after that.
#[cfg(feature = "gui")]
#[tauri::command]
pub fn set_window_title(window: tauri::WebviewWindow, title: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Not `with_webview`: on macOS it hands out the WKWebView and NSWindow
        // with references that are never released, so a window whose title was
        // set kept its page running, and its media playing, after it closed.
        let target = window.clone();
        window
            .run_on_main_thread(move || {
                let Ok(ns_window) = target.ns_window() else {
                    return;
                };
                let ns_window = ns_window.cast::<objc2_app_kit::NSWindow>();
                if ns_window.is_null() {
                    return;
                }
                // SAFETY: an autoreleased pointer used on the main thread while
                // the window is still open.
                let ns_window = unsafe { &*ns_window };
                ns_window.setTitle(&objc2_foundation::NSString::from_str(&title));
                if let Some(view) = ns_window.contentView() {
                    unsafe { view.setNeedsDisplay(true) };
                }
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        window.set_title(&title).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_live(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<(), String> {
    log::info!("Open player window: {room_id} {live_id}");
    // trim @ from room_id
    let clean_room_id = room_id.trim_start_matches('@');
    #[cfg(feature = "gui")]
    {
        use std::str::FromStr;

        let platform = PlatformType::from_str(&platform)?;
        let label = format!("Live:{clean_room_id}:{live_id}");

        // 窗口已存在时 build 会直接失败，这里改为聚焦到已有窗口，
        // 否则用户点击后没有任何反应。
        if let Some(window) = state.app_handle.get_webview_window(&label) {
            log::info!("Live window already exists, focusing: {label}");
            let _ = window.unminimize();
            let _ = window.show();
            return window
                .set_focus()
                .map_err(|e| format!("聚焦已有直播窗口失败：{e}"));
        }

        let builder = tauri::WebviewWindowBuilder::new(
            &state.app_handle,
            label.clone(),
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
        .title(format!("Live[{}] {}", room_id, live_id))
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

        if let Err(e) = with_custom_titlebar(builder).build() {
            log::error!("live window build failed: {e}");
            return Err(format!("打开直播窗口失败：{e}"));
        }
    }

    Ok(())
}

/// Focus the window `label`, or open `url` in a new one.
#[cfg(feature = "gui")]
fn open_or_focus_window(
    state: &State,
    label: String,
    url: String,
    title: String,
    size: (f64, f64),
) -> Result<(), String> {
    // 窗口已存在时 build 会直接失败，这里改为聚焦到已有窗口，
    // 否则用户重复点击时没有任何反应。
    if let Some(window) = state.app_handle.get_webview_window(&label) {
        log::info!("Window already exists, focusing: {label}");
        let _ = window.unminimize();
        let _ = window.show();
        return window
            .set_focus()
            .map_err(|e| format!("聚焦已有窗口失败：{e}"));
    }

    let builder = tauri::WebviewWindowBuilder::new(
        &state.app_handle,
        label,
        tauri::WebviewUrl::App(url.into()),
    )
    .title(title)
    .inner_size(size.0, size.1)
    .effects(WindowEffectsConfig {
        effects: vec![
            tauri_utils::WindowEffect::Tabbed,
            tauri_utils::WindowEffect::Mica,
        ],
        state: None,
        radius: None,
        color: None,
    });

    if let Err(e) = with_custom_titlebar(builder).build() {
        log::error!("window build failed: {e}");
        return Err(format!("打开窗口失败：{e}"));
    }
    Ok(())
}

/// Preview a material.
#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_clip(state: state_type!(), video_id: i64) -> Result<(), String> {
    log::info!("Open material window: {video_id}");
    open_or_focus_window(
        &state,
        format!("Clip:{video_id}"),
        format!("index_clip.html?material={video_id}"),
        format!("Material window:{video_id}"),
        (1000.0, 680.0),
    )
}

/// Edit a project.
#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_project(state: state_type!(), project_id: i64) -> Result<(), String> {
    log::info!("Open project window: {project_id}");
    open_or_focus_window(
        &state,
        format!("Project:{project_id}"),
        format!("index_clip.html?project={project_id}"),
        format!("Project window:{project_id}"),
        (1360.0, 860.0),
    )
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn list_folder(_state: state_type!(), path: String) -> Result<Vec<String>, String> {
    let path = PathBuf::from(path);
    let entries = std::fs::read_dir(path).map_err(|e| format!("Read directory failed: {e}"))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        files.push(entry.path().to_string_lossy().into_owned());
    }
    Ok(files)
}

#[cfg_attr(feature = "gui", tauri::command)]
#[allow(dead_code)]
pub async fn file_exists(_state: state_type!(), path: String) -> Result<bool, String> {
    let path = PathBuf::from(path);
    match std::fs::metadata(&path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(_) => Ok(false),
    }
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
