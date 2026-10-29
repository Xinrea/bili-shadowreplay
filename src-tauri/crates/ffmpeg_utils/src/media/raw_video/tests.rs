use super::*;
fn spec(frames: u64) -> RawVideoSpec {
    RawVideoSpec::bgra(3, 2, 30.0, frames, RawVideoAlpha::Premultiplied).unwrap()
}
#[test]
fn copies_packed_rows_before_recycling_and_validates_finished() {
    let (sender, input) = raw_video_channel(spec(1), 1).unwrap();
    let (recycle, returns) = std::sync::mpsc::sync_channel(1);
    let stats = Arc::new(Mutex::new(RawVideoStats::default()));
    let mut reader = input
        .with_recycle(recycle)
        .with_stats(stats.clone())
        .into_reader();
    let data = (0..24).collect::<Vec<u8>>();
    sender
        .blocking_send(Ok(RawVideoFrame::Frame(data.clone())))
        .unwrap();
    let frame = reader.next(&AtomicBool::new(false)).unwrap().unwrap();
    let mut recycled = returns.recv().unwrap();
    assert_eq!(recycled, data);
    recycled.fill(255);
    assert_eq!(frame.pts(), Some(0));
    assert_eq!(unsafe { (*frame.as_ptr()).duration }, 1);
    for (row, expected) in frame
        .data(0)
        .chunks(frame.stride(0))
        .take(2)
        .zip(data.chunks(12))
    {
        assert_eq!(&row[..12], expected);
    }
    #[cfg(bsr_ffmpeg_9_0)]
    assert_eq!(
        unsafe { (*frame.as_ptr()).alpha_mode },
        ffi::AVAlphaMode::AVALPHA_MODE_PREMULTIPLIED
    );
    sender.blocking_send(Ok(RawVideoFrame::Finished)).unwrap();
    assert!(reader.next(&AtomicBool::new(false)).unwrap().is_none());
    assert!(sender.is_closed());
    let stats = stats.lock().unwrap();
    assert_eq!((stats.frames, stats.bytes), (1, 24));
    assert!(stats.finished_at.is_some());
}
#[test]
fn malformed_count_size_missing_finish_and_producer_error_fail() {
    for (message, expected) in [
        (Ok(RawVideoFrame::Frame(vec![0; 23])), "size"),
        (Ok(RawVideoFrame::Finished), "0 of 1"),
        (Err("render failure".into()), "render failure"),
    ] {
        let (sender, input) = raw_video_channel(spec(1), 1).unwrap();
        let mut reader = input.into_reader();
        sender.blocking_send(message).unwrap();
        assert!(reader
            .next(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains(expected));
        assert!(sender.is_closed());
    }
    let (sender, input) = raw_video_channel(spec(1), 1).unwrap();
    let mut reader = input.into_reader();
    sender
        .blocking_send(Ok(RawVideoFrame::Frame(vec![0; 24])))
        .unwrap();
    reader.next(&AtomicBool::new(false)).unwrap();
    drop(sender);
    assert!(reader
        .next(&AtomicBool::new(false))
        .err()
        .unwrap()
        .contains("without Finished"));
}
#[test]
fn external_close_wakes_send_before_consumer_worker_starts() {
    let (sender, input) = raw_video_channel(spec(2), 1).unwrap();
    let close = input.cancel_handle();
    sender
        .blocking_send(Ok(RawVideoFrame::Frame(vec![0; 24])))
        .unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        assert!(sender
            .blocking_send(Ok(RawVideoFrame::Frame(vec![1; 24])))
            .is_err());
    });
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    let begin = Instant::now();
    close.close();
    worker.join().unwrap();
    assert!(begin.elapsed() < Duration::from_secs(1));
}
#[test]
fn global_cancel_wakes_waiting_receiver_and_consumer_drop_closes_recycle() {
    let (sender, input) = raw_video_channel(spec(1), 1).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let worker = std::thread::spawn(move || {
        let mut reader = input.into_reader();
        assert!(reader.next(&signal).is_err());
    });
    cancel.store(true, Ordering::Release);
    worker.join().unwrap();
    assert!(sender.is_closed());
    let (sender, input) = raw_video_channel(spec(1), 1).unwrap();
    let (recycle, returns) = std::sync::mpsc::sync_channel(1);
    drop(input.with_recycle(recycle));
    assert!(sender.is_closed());
    assert!(matches!(
        returns.recv_timeout(Duration::from_secs(1)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}
#[test]
fn external_close_also_releases_renderer_waiting_for_recycled_buffers() {
    let (sender, input) = raw_video_channel(spec(3), 3).unwrap();
    let close = input.cancel_handle();
    let (recycle, returns) = std::sync::mpsc::sync_channel(3);
    let input = input.with_recycle(recycle);
    let (ready, started) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        for _ in 0..3 {
            sender
                .blocking_send(Ok(RawVideoFrame::Frame(vec![0; 24])))
                .unwrap();
        }
        ready.send(()).unwrap();
        let mut waited = false;
        while !sender.is_closed() {
            match returns.recv_timeout(POLL) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => waited = true,
                _ => panic!("unstarted worker must still own the return channel"),
            }
        }
        assert!(waited);
    });
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    std::thread::sleep(Duration::from_millis(60));
    close.close();
    worker.join().unwrap();
    drop(input);
}
#[test]
fn shape_rate_duration_and_capacity_are_bounded() {
    for fps in [0.0, -1.0, 241.0, f64::NAN, f64::INFINITY] {
        assert!(RawVideoSpec::bgra(3, 2, fps, 1, RawVideoAlpha::Premultiplied).is_err());
    }
    for (width, height, count) in [
        (0, 1, 1),
        (8193, 1, 1),
        (8192, 8192, 1),
        (3, 2, 0),
        (3, 2, u64::MAX),
    ] {
        assert!(
            RawVideoSpec::bgra(width, height, 30.0, count, RawVideoAlpha::Premultiplied).is_err()
        );
    }
    assert!(raw_video_channel(spec(1), 0).is_err());
    assert!(raw_video_channel(spec(1), 4).is_err());
}
