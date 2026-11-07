use std::{
    collections::{btree_map::Entry, BTreeMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};

use fontdb::{Database, Family, Language, Source};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{state::State, state_type};

#[cfg(feature = "gui")]
use tauri::State as TauriState;

static SYSTEM_FONTS: OnceLock<Database> = OnceLock::new();
static FONT_FAMILIES: OnceLock<Vec<SubtitleFontFamily>> = OnceLock::new();

const FONT_CACHE_DIRECTORY: &str = "subtitle-fonts";
const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc"];

// Native subtitle rendering uses platform fonts for missing glyphs
// (DirectWrite, CoreText or fontconfig). Prefer the family each provider
// normally returns for Simplified Chinese, so the preview's only CJK fallback
// is the one used when burning.
#[cfg(target_os = "windows")]
const CJK_FAMILIES: &[&str] = &[
    "Microsoft YaHei",
    "DengXian",
    "SimHei",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
];
#[cfg(target_os = "macos")]
const CJK_FAMILIES: &[&str] = &[
    "PingFang SC",
    "Hiragino Sans GB",
    "Heiti SC",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const CJK_FAMILIES: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "WenQuanYi Zen Hei",
    "Droid Sans Fallback",
];

#[derive(Debug)]
struct CachedSubtitleFont {
    /// Path relative to the font cache root.
    file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleFontFamily {
    /// Name written to the ASS style; Skia resolves it using the encoding machine's fonts.
    family: String,
    /// Chinese family name shown beside `family`, when the font provides one.
    localized: Option<String>,
    /// Whether the font maps common Chinese characters itself.
    chinese: bool,
}

#[derive(Debug)]
struct FontFile {
    path: PathBuf,
}

fn system_fonts() -> &'static Database {
    SYSTEM_FONTS.get_or_init(|| {
        let mut database = Database::new();
        database.load_system_fonts();
        database
    })
}

fn is_supported_font(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            FONT_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn family_files(database: &Database, family: &str) -> Vec<FontFile> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for face in database.faces().filter(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
    }) {
        // This database only loads system files; caller input is never used as
        // a filesystem path. All faces/styles of the family remain available.
        let Source::File(path) = &face.source else {
            continue;
        };
        if !is_supported_font(path) || !seen.insert(path.clone()) {
            continue;
        }
        files.push(FontFile { path: path.clone() });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn sans_family_files(database: &Database) -> Vec<FontFile> {
    [
        database.family_name(&Family::SansSerif),
        "Arial",
        "Helvetica",
        "DejaVu Sans",
        "Liberation Sans",
        "Noto Sans",
    ]
    .iter()
    .find_map(|family| {
        let files = family_files(database, family);
        (!files.is_empty()).then_some(files)
    })
    .unwrap_or_default()
}

fn requested_family_files(database: &Database, requested: &str) -> Vec<FontFile> {
    if requested.eq_ignore_ascii_case("sans-serif") || requested.eq_ignore_ascii_case("sans") {
        sans_family_files(database)
    } else {
        family_files(database, requested)
    }
}

fn select_font_files(database: &Database, requested: &str) -> Vec<FontFile> {
    let mut selected: Vec<FontFile> = Vec::new();
    let mut add = |font: FontFile| {
        if !selected.iter().any(|other| other.path == font.path) {
            selected.push(font);
        }
    };

    // Keep the requested family first, then one explicit Chinese fallback and
    // the system sans face. Skia performs glyph/style selection from them.
    for font in requested_family_files(database, requested) {
        add(font);
    }
    if let Some(files) = CJK_FAMILIES
        .iter()
        .map(|family| family_files(database, family))
        .find(|files| !files.is_empty())
    {
        for font in files {
            add(font);
        }
    }
    for font in sans_family_files(database) {
        add(font);
    }
    selected
}

/// Names the style editor can write to an ASS `Fontname` field. Fonts whose
/// names start with `.` are hidden system UI faces.
fn is_ass_family_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['.', '*'])
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ',' | '\\' | '{' | '}' | '[' | ']'))
}

fn chinese_name_rank(language: Language) -> Option<u8> {
    match language {
        Language::Chinese_PeoplesRepublicOfChina => Some(0),
        Language::Chinese_Singapore => Some(1),
        Language::Chinese_Taiwan => Some(2),
        Language::Chinese_HongKongSAR => Some(3),
        Language::Chinese_MacaoSAR => Some(4),
        _ => None,
    }
}

/// Whether a face maps common Chinese characters. Only the table directory and
/// `cmap` are read, since CJK font files are often tens of megabytes.
fn covers_chinese(path: &Path, index: u32) -> bool {
    let read = || -> Option<bool> {
        let mut file = File::open(path).ok()?;
        let mut head = Vec::new();
        (&mut file).take(64 * 1024).read_to_end(&mut head).ok()?;
        let face = ttf_parser::RawFace::parse(&head, index).ok()?;
        let record = face
            .table_records
            .into_iter()
            .find(|record| record.tag == ttf_parser::Tag::from_bytes(b"cmap"))?;
        if record.length > 16 * 1024 * 1024 {
            return None;
        }
        file.seek(SeekFrom::Start(record.offset.into())).ok()?;
        let mut data = vec![0; record.length as usize];
        file.read_exact(&mut data).ok()?;
        let cmap = ttf_parser::cmap::Table::parse(&data)?;
        Some("中文字幕".chars().all(|character| {
            cmap.subtables.into_iter().any(|subtable| {
                subtable.is_unicode() && subtable.glyph_index(character.into()).is_some()
            })
        }))
    };
    read().unwrap_or(false)
}

fn font_families(database: &Database) -> Vec<SubtitleFontFamily> {
    let mut families = BTreeMap::new();
    for face in database.faces() {
        let Source::File(path) = &face.source else {
            continue;
        };
        let Some((family, _)) = face.families.first() else {
            continue;
        };
        if !is_supported_font(path) || !is_ass_family_name(family) {
            continue;
        }
        let localized = face
            .families
            .iter()
            .filter(|(name, _)| !name.eq_ignore_ascii_case(family))
            .filter_map(|(name, language)| Some((chinese_name_rank(*language)?, name)))
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, name)| name.clone());
        match families.entry(family.to_lowercase()) {
            Entry::Occupied(mut entry) => {
                let existing: &mut SubtitleFontFamily = entry.get_mut();
                if existing.localized.is_none() {
                    existing.localized = localized;
                }
            }
            // Faces of one family share coverage; check only the first file.
            Entry::Vacant(entry) => {
                entry.insert(SubtitleFontFamily {
                    family: family.clone(),
                    localized,
                    chinese: covers_chinese(path, face.index),
                });
            }
        }
    }
    families.into_values().collect()
}

/// List installed families on the machine doing the encoding, so the style
/// editor offers only fonts that both preview and burn-in can load.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_subtitle_font_families(
    _state: state_type!(),
) -> Result<Vec<SubtitleFontFamily>, String> {
    tokio::task::spawn_blocking(|| {
        FONT_FAMILIES
            .get_or_init(|| font_families(system_fonts()))
            .clone()
    })
    .await
    .map_err(|error| format!("读取系统字体列表失败：{error}"))
}

fn hex_digest(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    format!("{:x}", digest.finalize())
}

/// Copy a system font into a cache shared by every font name. Files are named
/// `<source>-<version>` so a changed system font gets a new cache identity, and the
/// previous copy of that source is removed.
fn cache_font(font: FontFile, cache_root: &Path) -> Result<CachedSubtitleFont, String> {
    let metadata =
        std::fs::metadata(&font.path).map_err(|error| format!("读取系统字幕字体失败：{error}"))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let source_key = hex_digest(&[font.path.as_os_str().as_encoded_bytes()]);
    let version = hex_digest(&[&metadata.len().to_le_bytes(), &modified.to_le_bytes()]);
    let extension = font
        .path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "系统字幕字体缺少文件扩展名".to_string())?
        .to_ascii_lowercase();
    let file_name = format!("{source_key}-{}.{extension}", &version[..16]);
    let directory = cache_root.join(FONT_CACHE_DIRECTORY);
    let destination = directory.join(&file_name);
    if !is_cached(&destination, metadata.len()) {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("创建字幕字体缓存失败：{error}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| format!("创建字幕字体缓存失败：{error}"))?;
        let mut source =
            File::open(&font.path).map_err(|error| format!("打开系统字幕字体失败：{error}"))?;
        std::io::copy(&mut source, temporary.as_file_mut())
            .map_err(|error| format!("缓存系统字幕字体失败：{error}"))?;
        if store_cached_font(temporary, &destination, metadata.len())? {
            remove_previous_versions(&directory, &source_key, &file_name);
        }
    }
    Ok(CachedSubtitleFont {
        file: format!("{FONT_CACHE_DIRECTORY}/{file_name}"),
    })
}

fn is_cached(path: &Path, length: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|cached| cached.len() == length)
}

/// Move a completed copy into place without replacing an existing entry.
/// Concurrent requests may copy the same font; the name encodes the source
/// size and mtime, so an entry of the right size is that same version. Never
/// replacing it also avoids Windows failing to replace a file FFmpeg is reading
/// through an encode directory's hard link. Returns whether this copy was used.
fn store_cached_font(
    temporary: tempfile::NamedTempFile,
    destination: &Path,
    length: u64,
) -> Result<bool, String> {
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(true),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if is_cached(destination, length) {
                return Ok(false);
            }
            // A damaged entry of the wrong size is replaced.
            error
                .file
                .persist(destination)
                .map_err(|error| format!("保存字幕字体缓存失败：{error}"))?;
            Ok(true)
        }
        Err(error) => Err(format!("保存字幕字体缓存失败：{}", error.error)),
    }
}

/// Best-effort cleanup: a stale copy may still be open on Windows (an encode
/// reading it through a hard link), which must not fail the current request.
fn remove_previous_versions(directory: &Path, source_key: &str, current: &str) {
    let prefix = format!("{source_key}-");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == current || !name.starts_with(&prefix) {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("清理过期字幕字体缓存失败：{error}"),
        }
    }
}

async fn prepare_subtitle_fonts(
    cache_root: PathBuf,
    font_name: String,
) -> Result<Vec<CachedSubtitleFont>, String> {
    let font_name = font_name.trim().to_string();
    if font_name.len() > 512 || font_name.contains('\0') {
        return Err("字幕字体名称无效".to_string());
    }
    tokio::task::spawn_blocking(move || {
        let fonts = select_font_files(system_fonts(), &font_name);
        if fonts.is_empty() {
            return Err("未找到可用的系统字幕字体，请安装中文字体后重启应用".to_string());
        }
        fonts
            .into_iter()
            .map(|font| cache_font(font, &cache_root))
            .collect()
    })
    .await
    .map_err(|error| format!("加载系统字幕字体失败：{error}"))?
}

/// Snapshot the selected font files for Skia rendering and cache identity.
/// Each job retains its own links even when the shared font cache is refreshed.
/// The directory is removed when the returned value is dropped.
pub async fn prepare_subtitle_font_directory(
    cache_root: PathBuf,
    font_name: String,
) -> Result<tempfile::TempDir, String> {
    let fonts = prepare_subtitle_fonts(cache_root.clone(), font_name).await?;
    tokio::task::spawn_blocking(move || link_font_directory(&cache_root, &fonts))
        .await
        .map_err(|error| format!("准备字幕字体失败：{error}"))?
}

fn link_font_directory(
    cache_root: &Path,
    fonts: &[CachedSubtitleFont],
) -> Result<tempfile::TempDir, String> {
    // Stay on the cache volume so hard links normally succeed without copying.
    let directory = tempfile::Builder::new()
        .prefix("encode-")
        .tempdir_in(cache_root.join(FONT_CACHE_DIRECTORY))
        .map_err(|error| format!("创建压制字体目录失败：{error}"))?;
    for font in fonts {
        let source = cache_root.join(&font.file);
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = directory.path().join(name);
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)
                .map_err(|error| format!("准备压制字体失败：{error}"))?;
        }
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_face(database: &mut Database, family: &str, path: &str, weight: fontdb::Weight) {
        database.push_face_info(fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: Source::File(PathBuf::from(path)),
            index: 0,
            families: vec![(family.to_string(), fontdb::Language::English_UnitedStates)],
            post_script_name: family.to_string(),
            style: fontdb::Style::Normal,
            weight,
            stretch: fontdb::Stretch::Normal,
            monospaced: false,
        });
    }

    /// A minimal sfnt whose only table is a format 12 `cmap` mapping `first..=last`.
    fn sfnt_with_cmap(first: u32, last: u32) -> Vec<u8> {
        let mut cmap = Vec::new();
        for value in [0u16, 1, 3, 10] {
            cmap.extend(value.to_be_bytes()); // version, tables, Windows, UCS-4
        }
        cmap.extend(12u32.to_be_bytes()); // subtable offset
        cmap.extend(12u16.to_be_bytes()); // format
        cmap.extend(0u16.to_be_bytes());
        for value in [28u32, 0, 1, first, last, 1] {
            cmap.extend(value.to_be_bytes()); // length, language, groups, group
        }
        let mut font = Vec::new();
        font.extend(0x0001_0000u32.to_be_bytes());
        font.extend(1u16.to_be_bytes());
        font.extend([0; 6]);
        font.extend(*b"cmap");
        for value in [0u32, 28, cmap.len() as u32] {
            font.extend(value.to_be_bytes()); // checksum, offset, length
        }
        font.extend(cmap);
        font
    }

    fn add_named_face(database: &mut Database, path: &Path, families: &[(&str, Language)]) {
        database.push_face_info(fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: Source::File(path.to_path_buf()),
            index: 0,
            families: families
                .iter()
                .map(|(name, language)| (name.to_string(), *language))
                .collect(),
            post_script_name: families[0].0.to_string(),
            style: fontdb::Style::Normal,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            monospaced: false,
        });
    }

    #[test]
    fn chinese_coverage_is_read_from_the_font_cmap() {
        let directory = tempfile::tempdir().unwrap();
        let cjk = directory.path().join("cjk.ttf");
        let latin = directory.path().join("latin.ttf");
        let broken = directory.path().join("broken.ttf");
        std::fs::write(&cjk, sfnt_with_cmap(0x4E00, 0x9FFF)).unwrap();
        std::fs::write(&latin, sfnt_with_cmap(0x20, 0x7E)).unwrap();
        std::fs::write(&broken, b"not a font").unwrap();
        assert!(covers_chinese(&cjk, 0));
        assert!(!covers_chinese(&cjk, 1));
        assert!(!covers_chinese(&latin, 0));
        assert!(!covers_chinese(&broken, 0));
        assert!(!covers_chinese(&directory.path().join("missing.ttf"), 0));
    }

    #[test]
    fn font_families_merge_faces_and_skip_names_ass_cannot_use() {
        let directory = tempfile::tempdir().unwrap();
        let cjk = directory.path().join("hei.ttc");
        let latin = directory.path().join("latin.ttf");
        std::fs::write(&cjk, sfnt_with_cmap(0x4E00, 0x9FFF)).unwrap();
        std::fs::write(&latin, sfnt_with_cmap(0x20, 0x7E)).unwrap();
        let english = Language::English_UnitedStates;
        let mut database = Database::new();
        add_named_face(
            &mut database,
            &cjk,
            &[
                ("Test Hei", english),
                ("測試黑體", Language::Chinese_Taiwan),
                ("测试黑体", Language::Chinese_PeoplesRepublicOfChina),
            ],
        );
        add_named_face(&mut database, &latin, &[("test hei", english)]);
        add_named_face(&mut database, &latin, &[("Arial", english)]);
        add_named_face(&mut database, &latin, &[(".SF NS", english)]);
        add_named_face(&mut database, &latin, &[("Bad, Name", english)]);
        add_named_face(
            &mut database,
            Path::new("/fonts/a.pfb"),
            &[("Type1", english)],
        );

        let families = font_families(&database);
        let names: Vec<_> = families.iter().map(|font| font.family.as_str()).collect();
        assert_eq!(names, ["Arial", "Test Hei"]);
        assert!(!families[0].chinese && families[0].localized.is_none());
        assert!(families[1].chinese);
        assert_eq!(families[1].localized.as_deref(), Some("测试黑体"));
    }

    fn font_file(path: &Path) -> FontFile {
        FontFile {
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn selects_all_family_variants_and_the_platform_cjk_fallback_without_duplicates() {
        let mut database = Database::new();
        add_face(
            &mut database,
            "Arial",
            "/fonts/regular.ttf",
            fontdb::Weight::NORMAL,
        );
        add_face(
            &mut database,
            "Arial",
            "/fonts/bold.ttf",
            fontdb::Weight::BOLD,
        );
        add_face(
            &mut database,
            "Arial",
            "/fonts/bold.ttf",
            fontdb::Weight::BOLD,
        );
        // Register the lower-priority family first: priority, not database
        // order, must decide which CJK fallback the preview receives.
        add_face(
            &mut database,
            CJK_FAMILIES[1],
            "/fonts/other.ttc",
            fontdb::Weight::NORMAL,
        );
        add_face(
            &mut database,
            CJK_FAMILIES[0],
            "/fonts/cjk.ttc",
            fontdb::Weight::NORMAL,
        );
        let fonts = select_font_files(&database, "arial");
        assert_eq!(fonts.len(), 3);
        let paths: Vec<_> = fonts.iter().map(|font| font.path.as_path()).collect();
        assert_eq!(
            paths,
            vec![
                Path::new("/fonts/bold.ttf"),
                Path::new("/fonts/regular.ttf"),
                Path::new("/fonts/cjk.ttc"),
            ]
        );
    }

    #[test]
    fn a_requested_cjk_family_is_not_duplicated_as_fallback() {
        let mut database = Database::new();
        add_face(
            &mut database,
            CJK_FAMILIES[0],
            "/fonts/cjk.ttc",
            fontdb::Weight::NORMAL,
        );
        let fonts = select_font_files(&database, CJK_FAMILIES[0]);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].path, Path::new("/fonts/cjk.ttc"));
    }

    #[test]
    fn unknown_or_path_like_names_resolve_only_database_fallbacks() {
        let mut database = Database::new();
        add_face(
            &mut database,
            "Arial",
            "/fonts/regular.ttf",
            fontdb::Weight::NORMAL,
        );
        let fonts = select_font_files(&database, "/private/secret.ttf");
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].path, Path::new("/fonts/regular.ttf"));
        assert!(requested_family_files(&database, "/private/secret.ttf").is_empty());
    }

    #[test]
    fn localized_and_generic_aliases_match_actual_family_files() {
        let mut database = Database::new();
        database.push_face_info(fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: Source::File(PathBuf::from("/fonts/yahei.ttc")),
            index: 0,
            families: vec![
                (
                    "Microsoft YaHei".to_string(),
                    fontdb::Language::English_UnitedStates,
                ),
                (
                    "微软雅黑".to_string(),
                    fontdb::Language::Chinese_PeoplesRepublicOfChina,
                ),
            ],
            post_script_name: "MicrosoftYaHei".to_string(),
            style: fontdb::Style::Normal,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            monospaced: false,
        });
        database.set_sans_serif_family("Microsoft YaHei");
        for requested in ["微软雅黑", "Microsoft YaHei", "sans-serif", "sans"] {
            let fonts = requested_family_files(&database, requested);
            assert_eq!(fonts.len(), 1, "{requested}");
            assert_eq!(fonts[0].path, Path::new("/fonts/yahei.ttc"));
        }
    }

    #[test]
    fn cache_is_shared_and_replaces_only_previous_versions_of_the_same_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.ttf");
        let other = directory.path().join("other.ttf");
        let cache = directory.path().join("cache");
        std::fs::write(&source, b"font-one").unwrap();
        std::fs::write(&other, b"other-font").unwrap();
        let first = cache_font(font_file(&source), &cache).unwrap();
        let repeated = cache_font(font_file(&source), &cache).unwrap();
        let unrelated = cache_font(font_file(&other), &cache).unwrap();
        assert_eq!(first.file, repeated.file);
        assert!(first.file.starts_with("subtitle-fonts/"));
        assert_eq!(std::fs::read(cache.join(&first.file)).unwrap(), b"font-one");

        std::fs::write(&source, b"font-two-longer").unwrap();
        let updated = cache_font(font_file(&source), &cache).unwrap();
        assert_ne!(first.file, updated.file);
        assert_eq!(
            std::fs::read(cache.join(&updated.file)).unwrap(),
            b"font-two-longer"
        );
        assert!(!cache.join(&first.file).exists());
        assert!(cache.join(&unrelated.file).exists());
    }

    #[test]
    fn a_concurrently_cached_copy_is_kept_and_a_damaged_entry_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("font.ttf");
        let copy = |content: &[u8]| {
            let mut file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
            std::io::Write::write_all(&mut file, content).unwrap();
            file
        };

        assert!(store_cached_font(copy(b"first"), &destination, 5).unwrap());
        // Another request finished first: keep its entry, discard this copy.
        assert!(!store_cached_font(copy(b"other"), &destination, 5).unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), b"first");
        // An entry whose size does not match its version is replaced.
        std::fs::write(&destination, b"bad").unwrap();
        assert!(store_cached_font(copy(b"fixed"), &destination, 5).unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixed");
        // Discarded copies do not leave temporary files behind.
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_requests_for_the_same_font_all_succeed() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.ttc");
        let cache = directory.path().join("cache");
        std::fs::write(&source, vec![7; 4 * 1024 * 1024]).unwrap();
        let barrier = std::sync::Barrier::new(8);
        let files: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        cache_font(font_file(&source), &cache).unwrap().file
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        assert!(files.iter().all(|file| file == &files[0]));
        let entries: Vec<_> = std::fs::read_dir(cache.join(FONT_CACHE_DIRECTORY))
            .unwrap()
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            std::fs::read(cache.join(&files[0])).unwrap().len(),
            4 * 1024 * 1024
        );
    }

    #[test]
    fn encode_font_directory_contains_only_the_selected_files_and_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        let fonts: Vec<_> = ["one.ttf", "two.ttc", "unused.ttf"]
            .iter()
            .map(|name| {
                let source = directory.path().join(name);
                std::fs::write(&source, name.as_bytes()).unwrap();
                cache_font(font_file(&source), &cache).unwrap()
            })
            .collect();
        let path;
        {
            let encode = link_font_directory(&cache, &fonts[..2]).unwrap();
            path = encode.path().to_path_buf();
            let mut names: Vec<_> = std::fs::read_dir(&path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            let mut expected: Vec<_> = fonts[..2]
                .iter()
                .map(|font| Path::new(&font.file).file_name().unwrap().to_owned())
                .collect();
            expected.sort();
            assert_eq!(names, expected);
            for font in &fonts[..2] {
                let name = Path::new(&font.file).file_name().unwrap();
                assert_eq!(
                    std::fs::read(path.join(name)).unwrap(),
                    std::fs::read(cache.join(&font.file)).unwrap()
                );
            }
        }
        assert!(!path.exists());
        assert!(cache.join(&fonts[0].file).exists());
    }
}
