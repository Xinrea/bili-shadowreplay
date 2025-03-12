use std::process::Command;

use tauri::Theme;
use tauri_utils::config::WindowEffectsConfig;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::recorder::PlatformType;
use crate::state::State;

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

#[tauri::command]
pub fn show_in_folder(path: String) {
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
            Command::new("xdg-open").arg(&new_path).spawn().unwrap();
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
                .unwrap();
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
    free: u64,
}

#[tauri::command]
pub async fn get_disk_info(state: tauri::State<'_, State>) -> Result<DiskInfo, ()> {
    let cache = state.config.read().await.cache.clone();
    // check system disk info
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // get cache disk info
    let mut disk_info = DiskInfo {
        disk: "".into(),
        total: 0,
        free: 0,
    };
    for disk in disks.list() {
        // if output is under disk mount point
        if cache.starts_with(disk.mount_point().to_str().unwrap()) {
            // if MacOS, using disk name
            #[cfg(target_os = "macos")]
            {
                disk_info.disk = disk.name().to_str().unwrap().into();
            }
            // if Windows, using disk mount point
            #[cfg(target_os = "windows")]
            {
                disk_info.disk = disk.mount_point().to_str().unwrap().into();
            }
            disk_info.total = disk.total_space();
            disk_info.free = disk.available_space();
            break;
        }
    }
    Ok(disk_info)
}

#[tauri::command]
pub async fn export_to_file(
    _state: tauri::State<'_, State>,
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
        return Err(format!("Write file failed: {}", e));
    }
    if let Err(e) = file.flush().await {
        return Err(format!("Flush file failed: {}", e));
    }
    Ok(())
}

#[tauri::command]
pub async fn open_live(
    state: tauri::State<'_, State>,
    platform: String,
    room_id: u64,
    live_id: String,
) -> Result<(), String> {
    log::info!("Open player window: {} {}", room_id, live_id);
    let addr = state.recorder_manager.get_hls_server_addr().await.unwrap();
    let platform = PlatformType::from_str(&platform).unwrap();
    let recorder_info = state
        .recorder_manager
        .get_recorder_info(platform, room_id)
        .await
        .unwrap();
    let handle = state.app_handle.clone();
    let builder = tauri::WebviewWindowBuilder::new(
        &handle,
        format!("Live:{}:{}", room_id, live_id),
        tauri::WebviewUrl::App(
            format!(
                "live_index.html?port={}&platform={}&room_id={}&live_id={}",
                addr.port(),
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
        log::error!("live window build failed: {}", e);
    }

    Ok(())
}
