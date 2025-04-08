use m3u8_rs::ExtTag;
use m3u8_rs::MediaPlaylistType;
use m3u8_rs::MediaSegment;
use std::io::Write;

pub struct HLSPlaylist {
    pub version: i64,
    pub target_duration: f32,
    pub playlist_type: MediaPlaylistType,
    pub media_sequence: i64,
    pub segments: Vec<MediaSegment>,
    pub end_list: bool,
    pub extra_tags: Vec<ExtTag>,
}

impl HLSPlaylist {
    pub fn new() -> Self {
        HLSPlaylist {
            version: 6,
            target_duration: 0.0,
            playlist_type: MediaPlaylistType::Vod,
            media_sequence: 0,
            segments: Vec::new(),
            end_list: false,
            extra_tags: Vec::new(),
        }
    }
}

impl ToString for HLSPlaylist {
    fn to_string(&self) -> String {
        let mut playlist = String::new();

        playlist.push_str("#EXTM3U\n");
        playlist.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));
        playlist.push_str(&format!(
            "#EXT-X-TARGETDURATION:{:.2}\n",
            self.target_duration
        ));

        playlist.push_str(&format!("#EXT-X-PLAYLIST-TYPE:{}\n", self.playlist_type));
        playlist.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence));
        for tag in &self.extra_tags {
            playlist.push_str(&format!("{}\n", tag));
        }

        for segment in &self.segments {
            let mut seg_str = vec![];
            if let Err(e) = segment_format(segment, &mut seg_str) {
                log::error!("Error formatting segment: {}", e);

                continue;
            }
            playlist.push_str(&String::from_utf8_lossy(&seg_str));
        }

        if self.end_list {
            playlist.push_str("#EXT-X-ENDLIST\n");
        }

        playlist
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
