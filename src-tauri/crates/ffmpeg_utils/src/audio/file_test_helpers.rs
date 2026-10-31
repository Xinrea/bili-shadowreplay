use super::*;

use std::{collections::BTreeMap, path::Path};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Chapter {
    start: i64,
    end: i64,
    time_base: (i32, i32),
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AudioFile {
    pub(super) codec: codec::Id,
    pub(super) rate: u32,
    pub(super) channels: u16,
    pub(super) extradata: Vec<u8>,
    pub(super) time_base: (i32, i32),
    pub(super) start: i64,
    pub(super) duration: i64,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) stream_metadata: BTreeMap<String, String>,
    pub(super) chapters: Vec<Chapter>,
    pub(super) packets: Vec<super::opus_tests::EncodedPacket>,
}

pub(super) fn snapshot(path: &Path) -> AudioFile {
    let mut input = open_input(path, Arc::new(AtomicBool::new(false))).unwrap();
    let stream = input.streams().best(media::Type::Audio).unwrap();
    assert_eq!(
        input
            .streams()
            .filter(|stream| stream.parameters().medium() == media::Type::Audio)
            .count(),
        1
    );
    assert!(
        !input.streams().any(|stream| matches!(
            stream.parameters().medium(),
            media::Type::Video | media::Type::Subtitle
        )),
        "audio output must not retain pictures or subtitles"
    );
    let parameters = stream.parameters();
    let raw = unsafe { &*parameters.as_ptr() };
    let extradata = if raw.extradata_size == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize) }.to_vec()
    };
    let decoder = codec::context::Context::from_parameters(parameters.clone())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    let index = stream.index();
    let mut result = AudioFile {
        codec: parameters.id(),
        rate: decoder.rate(),
        channels: decoder.channels(),
        extradata,
        time_base: (
            stream.time_base().numerator(),
            stream.time_base().denominator(),
        ),
        start: stream.start_time(),
        duration: stream.duration(),
        metadata: input
            .metadata()
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        stream_metadata: stream
            .metadata()
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        chapters: input
            .chapters()
            .map(|chapter| Chapter {
                start: chapter.start(),
                end: chapter.end(),
                time_base: (
                    chapter.time_base().numerator(),
                    chapter.time_base().denominator(),
                ),
                metadata: chapter
                    .metadata()
                    .iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            })
            .collect(),
        packets: Vec::new(),
    };
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {
                if packet.stream() != index {
                    continue;
                }
                result.packets.push(super::opus_tests::EncodedPacket {
                    data: packet.data().unwrap().to_vec(),
                    pts: packet.pts(),
                    dts: packet.dts(),
                    duration: packet.duration(),
                    skip: packet
                        .side_data()
                        .filter(|data| data.kind() == codec::packet::side_data::Type::SkipSamples)
                        .map(|data| data.data().to_vec())
                        .collect(),
                });
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("demux failed: {error}"),
        }
    }
    result
}

pub(super) fn assert_equivalent(actual: &AudioFile, expected: &AudioFile, case: &str) {
    assert_eq!(
        (actual.codec, actual.rate, actual.channels),
        (expected.codec, expected.rate, expected.channels),
        "{case}: codec profile"
    );
    assert_eq!(
        actual.extradata, expected.extradata,
        "{case}: codec configuration"
    );
    assert_eq!(
        (actual.time_base, actual.start, actual.duration),
        (expected.time_base, expected.start, expected.duration),
        "{case}: stream timeline"
    );
    assert_eq!(actual.metadata, expected.metadata, "{case}: global tags");
    assert_eq!(
        actual.stream_metadata, expected.stream_metadata,
        "{case}: audio tags"
    );
    assert_eq!(
        actual.chapters, expected.chapters,
        "{case}: chapter metadata"
    );
    assert_eq!(
        actual.packets.len(),
        expected.packets.len(),
        "{case}: packet count"
    );
    for (index, (actual, expected)) in actual.packets.iter().zip(&expected.packets).enumerate() {
        assert!(
            actual == expected,
            "{case}: packet {index} payload/PTS/DTS/duration/skip differ: {:?} vs {:?}",
            (actual.pts, actual.dts, actual.duration, &actual.skip),
            (
                expected.pts,
                expected.dts,
                expected.duration,
                &expected.skip
            )
        );
    }
}

#[derive(Default)]
struct Collector(Vec<i16>);
impl AudioSink for Collector {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        self.0.extend_from_slice(samples);
        Ok(())
    }
}

pub(super) async fn decoded_mono(path: &Path, rate: u32) -> Vec<i16> {
    let decoded = decode_audio(
        path,
        AudioDecodeOptions { sample_rate: rate },
        Collector::default(),
    )
    .await
    .unwrap();
    assert!(decoded.has_audio);
    assert_eq!(decoded.samples, decoded.sink.0.len() as u64);
    decoded.sink.0
}

pub(super) fn assert_samples(actual: &[i16], expected: &[i16], case: &str) {
    assert_eq!(actual.len(), expected.len(), "{case}: sample count");
    assert!(actual.iter().eq(expected), "{case}: decoded samples differ");
}
