use std::path::{Path, PathBuf};

/// 视频编辑工程生成器
///
/// 生成 Premiere Pro XML 格式的工程文件，剪映可以直接导入

pub struct VideoProject {
    pub project_name: String,
    pub project_path: PathBuf,
}

/// 视频片段信息
pub struct VideoClip {
    pub file_path: PathBuf,
    pub start_time: f64, // 源视频中的开始时间（秒）
    pub duration: f64,   // 片段时长（秒）
    pub width: i32,
    pub height: i32,
}

/// 生成 Premiere Pro XML 工程文件
pub async fn generate_premiere_xml(
    project_name: &str,
    clips: Vec<VideoClip>,
    output_dir: &Path,
) -> Result<VideoProject, String> {
    let xml_filename = format!("{}.xml", project_name);
    let xml_path = output_dir.join(&xml_filename);

    let canvas_width = clips.first().map(|c| c.width).unwrap_or(1920);
    let canvas_height = clips.first().map(|c| c.height).unwrap_or(1080);

    // 计算帧率（假设 30fps）
    let frame_rate = 30;
    let timebase = frame_rate;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE xmeml>\n");
    xml.push_str("<xmeml version=\"5\">\n");
    xml.push_str("  <sequence>\n");
    xml.push_str(&format!("    <name>{}</name>\n", escape_xml(project_name)));
    xml.push_str(&format!(
        "    <duration>{}</duration>\n",
        clips
            .iter()
            .map(|c| (c.duration * frame_rate as f64) as i64)
            .sum::<i64>()
    ));
    xml.push_str("    <rate>\n");
    xml.push_str(&format!("      <timebase>{}</timebase>\n", timebase));
    xml.push_str("      <ntsc>FALSE</ntsc>\n");
    xml.push_str("    </rate>\n");

    // 媒体部分
    xml.push_str("    <media>\n");
    xml.push_str("      <video>\n");
    xml.push_str("        <format>\n");
    xml.push_str("          <samplecharacteristics>\n");
    xml.push_str(&format!("            <width>{}</width>\n", canvas_width));
    xml.push_str(&format!("            <height>{}</height>\n", canvas_height));
    xml.push_str("          </samplecharacteristics>\n");
    xml.push_str("        </format>\n");

    // 视频轨道
    xml.push_str("        <track>\n");

    let mut timeline_position = 0i64;
    for (i, clip) in clips.iter().enumerate() {
        let clip_duration_frames = (clip.duration * frame_rate as f64) as i64;
        let clip_start_frames = (clip.start_time * frame_rate as f64) as i64;

        xml.push_str("          <clipitem id=\"");
        xml.push_str(&format!("clipitem-{}", i + 1));
        xml.push_str("\">\n");
        xml.push_str(&format!(
            "            <name>{}</name>\n",
            escape_xml(&clip.file_path.file_name().unwrap().to_string_lossy())
        ));
        xml.push_str(&format!(
            "            <start>{}</start>\n",
            timeline_position
        ));
        xml.push_str(&format!(
            "            <end>{}</end>\n",
            timeline_position + clip_duration_frames
        ));
        xml.push_str(&format!("            <in>{}</in>\n", clip_start_frames));
        xml.push_str(&format!(
            "            <out>{}</out>\n",
            clip_start_frames + clip_duration_frames
        ));

        // 文件引用
        xml.push_str("            <file id=\"");
        xml.push_str(&format!("file-{}", i + 1));
        xml.push_str("\">\n");
        xml.push_str(&format!(
            "              <name>{}</name>\n",
            escape_xml(&clip.file_path.file_name().unwrap().to_string_lossy())
        ));
        xml.push_str(&format!(
            "              <pathurl>{}</pathurl>\n",
            escape_xml(&format!(
                "file://localhost{}",
                clip.file_path.to_string_lossy()
            ))
        ));
        xml.push_str("              <rate>\n");
        xml.push_str(&format!(
            "                <timebase>{}</timebase>\n",
            timebase
        ));
        xml.push_str("                <ntsc>FALSE</ntsc>\n");
        xml.push_str("              </rate>\n");
        xml.push_str("              <media>\n");
        xml.push_str("                <video>\n");
        xml.push_str("                  <samplecharacteristics>\n");
        xml.push_str(&format!(
            "                    <width>{}</width>\n",
            clip.width
        ));
        xml.push_str(&format!(
            "                    <height>{}</height>\n",
            clip.height
        ));
        xml.push_str("                  </samplecharacteristics>\n");
        xml.push_str("                </video>\n");
        xml.push_str("              </media>\n");
        xml.push_str("            </file>\n");

        xml.push_str("          </clipitem>\n");

        timeline_position += clip_duration_frames;
    }

    xml.push_str("        </track>\n");
    xml.push_str("      </video>\n");
    xml.push_str("    </media>\n");
    xml.push_str("  </sequence>\n");
    xml.push_str("</xmeml>\n");

    tokio::fs::write(&xml_path, xml)
        .await
        .map_err(|e| format!("Failed to write XML file: {}", e))?;

    Ok(VideoProject {
        project_name: project_name.to_string(),
        project_path: xml_path,
    })
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
