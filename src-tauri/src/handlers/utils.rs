use std::path::PathBuf;

use base64::Engine;

use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use {
    recorder::platforms::PlatformType, std::process::Command, tauri::Manager,
    tauri::State as TauriState, tauri_utils::config::WindowEffectsConfig, tokio::fs::OpenOptions,
    tokio::io::AsyncWriteExt,
};

/// Bring the main window to the front, from a preview window that asked it to
/// continue something there.
pub(crate) fn focus_main_window(_state: &State) {
    #[cfg(feature = "gui")]
    crate::tray::show_main_window(&_state.app_handle);
}

/// The bytes of the base64 data URL `data`.
pub(crate) fn decode_data_url(data: &str) -> Result<Vec<u8>, String> {
    let base64 = data
        .split("base64,")
        .nth(1)
        .ok_or_else(|| "Image is not a base64 data URL".to_string())?;
    base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| format!("Failed to decode image: {e}"))
}

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
    pub(crate) disk: String,
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

    // Off the async runtime: these are blocking calls.
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || file_system_info(&target))
            .await
            .map_err(|e| log::error!("Disk info task failed: {e}"))?
    }

    #[cfg(target_os = "windows")]
    {
        tokio::task::spawn_blocking(move || disk_info_of(&target))
            .await
            .map_err(|e| log::error!("Disk info task failed: {e}"))
    }
}

/// The file system holding `target`, or the nearest folder above it that
/// exists. Only it: listing every volume waits on each, as on a network share
/// that does not answer.
#[cfg(target_os = "macos")]
fn file_system_info(target: &std::path::Path) -> Result<DiskInfo, ()> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    let target = target
        .ancestors()
        .find(|path| path.exists())
        .unwrap_or(target);
    let path = CString::new(target.as_os_str().as_bytes())
        .map_err(|e| log::error!("Invalid path for disk info: {e}"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a valid C string, and `stats` is written in full when
    // the call succeeds.
    if unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        log::error!(
            "Failed to read disk info of {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        );
        return Err(());
    }
    // SAFETY: the call succeeded.
    let stats = unsafe { stats.assume_init() };
    let block = u64::from(stats.f_bsize);
    // SAFETY: the kernel writes the mount point as a NUL-terminated string.
    let mount_point = unsafe { CStr::from_ptr(stats.f_mntonname.as_ptr()) };
    Ok(DiskInfo {
        disk: mount_point.to_string_lossy().into_owned(),
        total: stats.f_blocks * block,
        free: stats.f_bavail * block,
    })
}

/// The disk with the longest mount point holding `target`.
#[cfg(target_os = "windows")]
fn disk_info_of(target: &std::path::Path) -> DiskInfo {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut disk_info = DiskInfo {
        disk: String::new(),
        total: 0,
        free: 0,
    };
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
    disk_info
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

/// Open the configured material directory itself, rather than reveal it in its parent.
#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_material_folder(state: state_type!()) -> Result<(), String> {
    let output = state.config.read().await.output.clone();
    let path = std::path::absolute(output)
        .map_err(|e| format!("Failed to resolve material directory: {e}"))?;
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| format!("Failed to create material directory: {e}"))?;

    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");

    command
        .arg(path)
        .spawn()
        .map_err(|e| format!("Failed to open material directory: {e}"))?;
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
    min_size: Option<(f64, f64)>,
    html5_drag_drop: bool,
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

    let mut builder = tauri::WebviewWindowBuilder::new(
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
    if let Some((width, height)) = min_size {
        builder = builder.min_inner_size(width, height);
    }
    if html5_drag_drop {
        // Windows' native file-drop handler otherwise intercepts material-bin
        // drags before the editor's HTML5 drop targets receive them.
        builder = builder.disable_drag_drop_handler();
    }

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
        None,
        false,
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
        // Material library, preview and side panel stay side by side.
        Some((1100.0, 680.0)),
        true,
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
    fn decode_data_url_reads_base64_payloads_only() {
        use super::decode_data_url;

        assert_eq!(
            decode_data_url("data:image/png;base64,aGVsbG8=").unwrap(),
            b"hello"
        );
        assert!(decode_data_url("data:image/png,hello").is_err());
        assert!(decode_data_url("data:image/png;base64,invalid!").is_err());
    }

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

#[cfg(all(test, target_os = "macos"))]
mod disk_info_tests {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    #[tokio::test]
    async fn disk_info_reads_the_file_system_of_a_folder_to_be() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("to/be");
        assert!(!target.exists());

        // Read the existing parent directly to verify that the missing child
        // resolves to its file system, not another volume selected by prefix.
        let path = CString::new(directory.path().as_os_str().as_bytes()).unwrap();
        let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `path` is a valid C string and `stats` is writable.
        assert_eq!(
            unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) },
            0
        );
        // SAFETY: the successful call initialized `stats`, including the
        // NUL-terminated mount point.
        let stats = unsafe { stats.assume_init() };
        let mount_point = unsafe { CStr::from_ptr(stats.f_mntonname.as_ptr()) };

        let info = get_disk_info_inner(target.clone()).await.unwrap();
        assert!(
            !target.exists(),
            "querying disk info must not create the path"
        );
        assert_eq!(info.disk, mount_point.to_string_lossy());
        assert_eq!(info.total, stats.f_blocks * u64::from(stats.f_bsize));
        // Free space can change between reads or be zero on a full volume.
        // sysinfo's macOS estimate also includes reclaimable capacity, so it
        // is not an oracle for statfs's immediately available blocks.
        assert!(info.total > 0 && info.free <= info.total);
    }
}
