use super::*;

fn video(id: u8, pts: i64, duration: i64) -> frame::Video {
    let mut frame = frame::Video::new(ffmpeg::format::Pixel::YUV420P, 16, 16);
    frame.data_mut(0).fill(id);
    frame.set_pts(Some(pts));
    unsafe {
        (*frame.as_mut_ptr()).duration = duration;
        (*frame.as_mut_ptr()).time_base = ffi::AVRational { num: 1, den: 1000 };
    }
    frame
}
fn sync(rate: i32, policy: FrameRatePolicy) -> FrameSync {
    FrameSync::new(
        policy,
        (rate, 1).into(),
        (1, 1000).into(),
        (1, rate).into(),
        0,
        0,
    )
    .unwrap()
}
fn collect(sync: &mut FrameSync, pts: &[i64]) -> Vec<(i64, i64, u8)> {
    let mut output = Vec::new();
    for (id, pts) in pts.iter().enumerate() {
        output.extend(
            sync.push(Some(&video(id as u8, *pts, 100)))
                .unwrap()
                .map(|frame| {
                    let frame = frame.unwrap();
                    (
                        frame.pts().unwrap(),
                        unsafe { (*frame.as_ptr()).duration },
                        frame.data(0)[0],
                    )
                }),
        );
    }
    output.extend(sync.push(None).unwrap().map(|frame| {
        let frame = frame.unwrap();
        (
            frame.pts().unwrap(),
            unsafe { (*frame.as_ptr()).duration },
            frame.data(0)[0],
        )
    }));
    output
}

#[test]
fn fpsconv_cfr_gap_and_eof_tail_match_cli_golden_frames() {
    // Independent n9 CLI rawvideo framehash baselines, verified by a scalar
    // C harness against n6.1/n9's FPSConv logic. Values identify source
    // pixels, including which previous frame fills gaps and the EOF tail.
    for (pts, rate, ids) in [
        (vec![0, 100, 200, 300, 400, 500], 10, vec![0, 1, 2, 3, 4, 5]),
        (
            vec![0, 100, 200, 600, 700, 800],
            10,
            vec![0, 1, 2, 2, 2, 3, 3, 4, 5],
        ),
        (
            vec![0, 250, 500, 750, 1000, 1250],
            10,
            vec![0, 0, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4, 5, 5, 5],
        ),
        (
            vec![0, 100, 200, 300, 400, 500],
            30,
            vec![0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 5],
        ),
    ] {
        let actual = collect(
            &mut sync(
                rate,
                FrameRatePolicy::Constant {
                    num: rate as u32,
                    den: 1,
                },
            ),
            &pts,
        );
        assert_eq!(
            actual,
            (0..ids.len())
                .map(|index| (index as i64, 1, ids[index]))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn fpsconv_half_tick_output_seek_and_nearest_even_are_preserved() {
    let mut sync = FrameSync::new(
        FrameRatePolicy::Constant { num: 10, den: 1 },
        (10, 1).into(),
        (1, 1000).into(),
        (1, 10).into(),
        0,
        150000,
    )
    .unwrap();
    let actual = collect(&mut sync, &[200, 300, 400, 500]);
    // Equivalent old -ss .15 -t .35: the encoder rejects the final PTS>=.35.
    assert_eq!(
        actual
            .into_iter()
            .filter(|frame| frame.0 < 4)
            .collect::<Vec<_>>(),
        vec![(0, 1, 0), (1, 1, 0), (2, 1, 1), (3, 1, 2)]
    );
    assert_eq!(
        (
            rounded(0.5).unwrap(),
            rounded(1.5).unwrap(),
            rounded(-0.5).unwrap()
        ),
        (0, 2, 0)
    );
    assert_eq!(
        (rounded_single(2.5).unwrap(), rounded_single(3.5).unwrap()),
        (2, 4)
    );
}

#[test]
fn fpsconv_auto_flags_avi_and_initial_offset_follow_cli_selection() {
    let pts = [300, 400, 500, 600, 700, 800];
    let full = collect(&mut sync(10, FrameRatePolicy::Auto), &pts);
    assert_eq!(full.len(), 9);
    let offset = collect(
        &mut sync(10, FrameRatePolicy::Auto).preserve_initial_gap(true),
        &pts,
    );
    assert_eq!(
        offset.iter().map(|frame| frame.0).collect::<Vec<_>>(),
        vec![3, 4, 5, 6, 7, 8]
    );
    let avi = collect(&mut sync(10, FrameRatePolicy::Auto).avi_auto(true), &pts);
    assert_eq!(avi, offset);
    for policy in [FrameRatePolicy::Variable, FrameRatePolicy::Passthrough] {
        let actual = collect(&mut sync(10, policy), &[0, 250, 500, 750, 1000, 1250]);
        assert_eq!(
            actual.iter().map(|frame| frame.0).collect::<Vec<_>>(),
            vec![0, 3, 5, 8, 10, 13]
        );
        assert_eq!(
            actual.iter().map(|frame| frame.2).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
    }
    let mut variable = FrameSync::new(
        FrameRatePolicy::Auto,
        (10, 1).into(),
        (1, 1000).into(),
        (1, 10).into(),
        ffi::AVFMT_VARIABLE_FPS,
        0,
    )
    .unwrap();
    assert_eq!(collect(&mut variable, &pts), offset);
}

#[test]
fn fpsconv_repeats_share_pixels_and_long_gaps_are_lazy_or_skipped() {
    let source = video(7, 0, 100);
    let mut sync = sync(30, FrameRatePolicy::Constant { num: 30, den: 1 });
    for repeated in sync.push(Some(&source)).unwrap() {
        let repeated = repeated.unwrap();
        assert_eq!(repeated.data(0).as_ptr(), source.data(0).as_ptr());
    }
    let long_gap = video(8, 10000000, 100);
    let mut batch = sync.push(Some(&long_gap)).unwrap();
    assert!(batch.count > 200000);
    assert!(batch.next().unwrap().is_ok());
    assert!(batch.next().unwrap().is_ok());
    drop(batch);
    let excessive = video(9, 1000000000, 100);
    assert!(sync.push(Some(&excessive)).unwrap().next().is_none());
}

#[test]
fn decoder_clock_fills_missing_pts_without_changing_actual_duration() {
    let mut clock = VideoClock::new((1, 1000).into(), (25, 1).into(), (0, 1).into(), true);
    let mut frame = frame::Video::empty();
    clock.normalize(&mut frame).unwrap();
    assert_eq!(frame.pts(), Some(0));
    assert_eq!(unsafe { (*frame.as_ptr()).duration }, 0);
    clock.normalize(&mut frame).unwrap();
    assert_eq!(frame.pts(), Some(40));
    clock.set_decoder_rate((50, 1).into());
    clock.normalize(&mut frame).unwrap();
    assert_eq!(frame.pts(), Some(80));
    clock.normalize(&mut frame).unwrap();
    assert_eq!(frame.pts(), Some(100));
    let mut clock = VideoClock::new((1, 1000).into(), (25, 1).into(), (0, 1).into(), false);
    unsafe {
        (*frame.as_mut_ptr()).best_effort_timestamp = 0;
        (*frame.as_mut_ptr()).duration = 1;
    }
    clock.normalize(&mut frame).unwrap();
    unsafe {
        (*frame.as_mut_ptr()).best_effort_timestamp = 100;
    }
    clock.normalize(&mut frame).unwrap();
    unsafe {
        (*frame.as_mut_ptr()).best_effort_timestamp = ffi::AV_NOPTS_VALUE;
    }
    clock.normalize(&mut frame).unwrap();
    assert_eq!(
        frame.pts(),
        Some(if ffmpeg::codec::version() >> 16 >= 63 {
            200
        } else {
            101
        })
    );
}

fn framehash(filter: &str, rate: Option<i32>, mode: &str, trim: bool) -> Vec<(i64, i64, String)> {
    let mut command = std::process::Command::new(crate::ffmpeg_path());
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "nullsrc=size=16x16:rate=10:duration=0.6,geq=lum='N*30':cb=128:cr=128",
        "-vf",
        filter,
    ]);
    if let Some(rate) = rate {
        command.args(["-r", &rate.to_string()]);
    }
    command.args(["-fps_mode", mode]);
    if trim {
        command.args(["-ss", "0.15", "-t", "0.35"]);
    }
    let result = command
        .args([
            "-c:v",
            "png",
            "-pix_fmt",
            "rgb24",
            "-threads",
            "1",
            "-f",
            "framehash",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "clock oracle: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            (
                fields[2].parse().unwrap(),
                fields[3].parse().unwrap(),
                fields[5].to_owned(),
            )
        })
        .collect()
}

#[test]
fn fpsconv_matches_cli_frame_pts_duration_and_pixel_identity_across_25_schedules() {
    if !std::process::Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skip FPSConv CLI oracle: ffmpeg unavailable");
        return;
    }
    let identity = framehash("settb=1/1000,setpts=PTS", None, "passthrough", false)
        .into_iter()
        .enumerate()
        .map(|(id, frame)| (frame.2, id as u8))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        identity.len(),
        6,
        "each generated source frame must be identifiable by its pixel hash"
    );
    for (name, expression, pts, start) in [
        ("plain", "PTS", [0, 100, 200, 300, 400, 500], 0),
        ("gap", "PTS+gte(N\\,3)*300", [0, 100, 200, 600, 700, 800], 0),
        ("shift", "PTS+300", [300, 400, 500, 600, 700, 800], 0),
        ("EOF tail", "PTS*5/2", [0, 250, 500, 750, 1000, 1250], 0),
        ("output trim", "PTS", [0, 100, 200, 300, 400, 500], 150000),
    ] {
        for (rate, policy, mode) in [
            (10, FrameRatePolicy::Constant { num: 10, den: 1 }, "cfr"),
            (25, FrameRatePolicy::Constant { num: 25, den: 1 }, "cfr"),
            (30, FrameRatePolicy::Constant { num: 30, den: 1 }, "cfr"),
            (10, FrameRatePolicy::Variable, "vfr"),
            (10, FrameRatePolicy::Passthrough, "passthrough"),
        ] {
            let mut sync = FrameSync::new(
                policy,
                (rate, 1).into(),
                (1, 1000).into(),
                (1, rate).into(),
                0,
                start,
            )
            .unwrap();
            let mut actual = Vec::new();
            for (id, pts) in pts
                .into_iter()
                .enumerate()
                .filter(|(_, pts)| *pts * 1000 >= start)
            {
                actual.extend(
                    sync.push(Some(&video(id as u8, pts, 100)))
                        .unwrap()
                        .map(|frame| {
                            let frame = frame.unwrap();
                            (
                                frame.pts().unwrap(),
                                unsafe { (*frame.as_ptr()).duration },
                                frame.data(0)[0],
                            )
                        }),
                );
            }
            actual.extend(sync.push(None).unwrap().map(|frame| {
                let frame = frame.unwrap();
                (
                    frame.pts().unwrap(),
                    unsafe { (*frame.as_ptr()).duration },
                    frame.data(0)[0],
                )
            }));
            // As in ffmpeg_enc.c check_recording_time, reject frames whose
            // post-synchronization PTS reaches -t; never shorten a frame.
            if start > 0 {
                actual.retain(|frame| unsafe { ffi::av_compare_ts(frame.0,ffi::AVRational {num:1,den:rate},350000,ffi::AVRational {num:1,den:ffi::AV_TIME_BASE}) } < 0);
            }
            let expected = framehash(
                &format!("settb=1/1000,setpts={expression}"),
                (mode == "cfr").then_some(rate),
                mode,
                start > 0,
            )
            .into_iter()
            .map(|(pts, duration, hash)| (pts, duration, identity[&hash]))
            .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "{name}/{rate}/{mode}: source identity, PTS and duration"
            );
        }
    }
}
