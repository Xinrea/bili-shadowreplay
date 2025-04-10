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
    pub extra_tags: Vec<ExtTag>,
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
            extra_tags: Vec::new(),
        }
    }

    pub fn from(p: &MediaPlaylist) -> Self {
        HLSPlaylist {
            version: p.version.unwrap_or(6),
            target_duration: p.target_duration,
            playlist_type: p.playlist_type.clone().unwrap_or(MediaPlaylistType::Vod),
            media_sequence: p.media_sequence,
            segments: p.segments.clone(),
            end_list: p.end_list,
            extra_tags: p.unknown_tags.clone(),
        }
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

    pub fn last_segment_time(&self) -> i64 {
        if let Some(last) = self.segments.last() {
            last.program_date_time.map(|dt| dt.timestamp()).unwrap_or(0)
        } else {
            0
        }
    }

    pub fn setup_danmu_offset_info(&mut self) {
        if let Some(first_segment) = self.segments.first() {
            if let Some(ts) = first_segment.program_date_time {
                if self.extra_tags.iter().any(|tag| tag.tag == "X-OFFSET") {
                    log::debug!("X-OFFSET already exists");
                    return;
                }
                self.extra_tags.push(ExtTag {
                    tag: "X-OFFSET".to_string(),
                    rest: Some(format!("{}", ts.timestamp())),
                });
            }
        }
    }

    pub fn update_last_sequence(&mut self, seq: u64) {
        if let Some(tag) = self
            .extra_tags
            .iter_mut()
            .find(|tag| tag.tag == "X-LASTSEQ")
        {
            tag.rest = Some(format!("{}", seq));
        } else {
            self.extra_tags.push(ExtTag {
                tag: "X-LASTSEQ".to_string(),
                rest: Some(format!("{}", seq)),
            });
        }
    }

    pub fn last_sequence(&self) -> Option<u64> {
        // find last_sequence in extra_tags
        for tag in &self.extra_tags {
            if tag.tag == "X-LASTSEQ" {
                if let Some(rest) = &tag.rest {
                    if let Ok(seq) = rest.parse::<u64>() {
                        return Some(seq);
                    }
                }
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
        f.write_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence))?;
        for tag in &self.extra_tags {
            f.write_str(&format!("{}\n", tag))?;
        }
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
