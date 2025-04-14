use m3u8_rs::ExtTag;
use m3u8_rs::MediaPlaylist;
use m3u8_rs::MediaPlaylistType;
use m3u8_rs::MediaSegment;
use std::fmt;
use std::fmt::Display;
use std::io::Write;

#[derive(Debug, Clone)]
pub struct HLSPlaylist {
    pub version: usize,
    pub target_duration: f32,
    pub playlist_type: MediaPlaylistType,
    pub media_sequence: u64,
    pub segments: Vec<MediaSegment>,
    pub end_list: bool,
}

impl HLSPlaylist {
    pub fn new() -> Self {
        HLSPlaylist {
            version: 6,
            target_duration: 1.0,
            playlist_type: MediaPlaylistType::Vod,
            media_sequence: 0,
            segments: Vec::new(),
            end_list: false,
        }
    }

    pub fn from(p: &MediaPlaylist) -> Self {
        log::debug!("Converting MediaPlaylist to HLSPlaylist: {:?}", p);
        HLSPlaylist {
            version: p.version.unwrap_or(6),
            target_duration: p.target_duration,
            playlist_type: p.playlist_type.clone().unwrap_or(MediaPlaylistType::Vod),
            media_sequence: p.media_sequence,
            segments: p.segments.clone(),
            end_list: p.end_list,
        }
    }

    pub fn get_live_id(&self) -> Option<String> {
        if let Some(header) = self.get_header() {
            // extract live ID from the header, example: h12345.m4s, find out 12345
            if let Some(start) = header.find("h") {
                if let Some(end) = header[start..].find(".m4s") {
                    return Some(header[start + 1..start + end].to_string());
                }
            }
        }

        log::warn!("No live ID found");

        None
    }

    pub fn get_header(&self) -> Option<String> {
        if !self.segments.is_empty() {
            let first_segment = self.segments.first().unwrap();
            if let Some(m) = first_segment.map.clone() {
                return Some(m.uri.clone());
            }
        }

        log::warn!("No header found");

        None
    }

    pub fn append_segement(&mut self, s: MediaSegment) {
        self.segments.push(s);
    }

    pub fn total_duration(&self) -> f32 {
        self.segments.iter().map(|s| s.duration).sum()
    }

    pub fn setup_danmu_offset_info(&mut self) {
        if let Some(first_segment) = self.segments.first_mut() {
            if let Some(ts) = first_segment.program_date_time {
                if first_segment
                    .unknown_tags
                    .iter()
                    .any(|tag| tag.tag == "X-OFFSET")
                {
                    return;
                }
                first_segment.unknown_tags.push(ExtTag {
                    tag: "X-OFFSET".to_string(),
                    rest: Some(format!("{}", ts.timestamp())),
                });
            }
        }
    }

    pub fn update_last_sequence(&mut self, seq: u64) {
        if let Some(first_segment) = self.segments.first_mut() {
            if !first_segment
                .unknown_tags
                .iter()
                .any(|tag| tag.tag == "X-LASTSEQ")
            {
                first_segment.unknown_tags.push(ExtTag {
                    tag: "X-LASTSEQ".to_string(),
                    rest: Some(format!("{}", seq)),
                });
            } else {
                for tag in &mut first_segment.unknown_tags {
                    if tag.tag == "X-LASTSEQ" {
                        tag.rest = Some(format!("{}", seq));
                    }
                }
            }
        }
    }

    pub fn last_sequence(&self) -> Option<u64> {
        if let Some(first_segment) = self.segments.first() {
            if let Some(tag) = first_segment
                .unknown_tags
                .iter()
                .find(|tag| tag.tag == "X-LASTSEQ")
            {
                return tag.rest.as_ref().and_then(|s| s.parse::<u64>().ok());
            }
        }

        None
    }

    pub fn output(&self, end: bool) -> String {
        let mut playlist = self.clone();
        playlist.playlist_type = if end {
            MediaPlaylistType::Vod
        } else {
            MediaPlaylistType::Event
        };
        playlist.setup_danmu_offset_info();
        playlist.end_list = end;

        playlist.to_string()
    }
}

impl Display for HLSPlaylist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("#EXTM3U\n")?;
        f.write_str(&format!("#EXT-X-VERSION:{}\n", self.version))?;
        f.write_str(&format!(
            "#EXT-X-TARGETDURATION:{}\n",
            self.target_duration as i64
        ))?;
        f.write_str(&format!("#EXT-X-PLAYLIST-TYPE:{}\n", self.playlist_type))?;
        f.write_str(&format!(
            "#EXT-X-MEDIA-SEQUENCE:{}\n\n",
            self.media_sequence
        ))?;

        for segment in &self.segments {
            let mut seg_str = vec![];
            if let Err(e) = segment_format(segment, &mut seg_str) {
                log::error!("Error formatting segment: {}", e);

                continue;
            }
            f.write_str(&String::from_utf8_lossy(&seg_str))?;
        }
        if self.end_list {
            f.write_str("#EXT-X-ENDLIST\n")?;
        }
        Ok(())
    }
}

fn segment_format<T: Write>(segment: &MediaSegment, w: &mut T) -> std::io::Result<()> {
    if let Some(ref byte_range) = segment.byte_range {
        write!(w, "#EXT-X-BYTERANGE:")?;
        byte_range.write_value_to(w)?;
        writeln!(w)?;
    }
    if segment.discontinuity {
        writeln!(w, "#EXT-X-DISCONTINUITY")?;
    }
    if let Some(ref key) = segment.key {
        write!(w, "#EXT-X-KEY:")?;
        key.write_attributes_to(w)?;
        writeln!(w)?;
    }
    if let Some(ref map) = segment.map {
        write!(w, "#EXT-X-MAP:")?;
        map.write_attributes_to(w)?;
        writeln!(w)?;
    }
    if let Some(ref v) = segment.program_date_time {
        writeln!(
            w,
            "#EXT-X-PROGRAM-DATE-TIME:{}",
            v.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        )?;
    }
    if let Some(ref v) = segment.daterange {
        write!(w, "#EXT-X-DATERANGE:")?;
        v.write_attributes_to(w)?;
        writeln!(w)?;
    }
    for unknown_tag in &segment.unknown_tags {
        writeln!(w, "{}", unknown_tag)?;
    }

    write!(w, "#EXTINF:{},", segment.duration)?;

    if let Some(ref v) = segment.title {
        writeln!(w, "{}", v)?;
    } else {
        writeln!(w)?;
    }

    writeln!(w, "{}", segment.uri)
}
