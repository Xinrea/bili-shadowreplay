#pragma once
#if defined(_WIN32) && !defined(NOMINMAX)
#define NOMINMAX
#endif
extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersink.h>
#include <libavfilter/buffersrc.h>
#include <libavutil/display.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>
#ifdef BSR_METAL
#include <libavutil/hwcontext_videotoolbox.h>
#else
#include <libavutil/hwcontext_vulkan.h>
#endif
}
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <deque>
#include <exception>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#include "bridge.h"
#include "ffmpeg_api.h"

#ifdef __OBJC__
#define BSR_POOL @autoreleasepool
#else
#define BSR_POOL
#endif

namespace {
using Clock = std::chrono::steady_clock;
constexpr size_t FRAME_SLOTS = 3;
static std::atomic<int> interrupted{0};
static_assert(std::atomic<int>::is_always_lock_free);
static void interruptSignal(int value) { interrupted.store(value, std::memory_order_relaxed); }
static void require(bool value, const std::string& message) { if (!value) throw std::runtime_error(message); }
static void checkInterrupted() { require(!interrupted.load(std::memory_order_relaxed), "export cancelled"); }
static void check(int value, const char* message) {
    if (value < 0) {
        char text[256]; av_strerror(value, text, sizeof(text));
        throw std::runtime_error(std::string(message) + ": " + text);
    }
}
struct FrameFree { void operator()(AVFrame* p) const { av_frame_free(&p); } };
struct PacketFree { void operator()(AVPacket* p) const { av_packet_free(&p); } };
struct CodecFree { void operator()(AVCodecContext* p) const { avcodec_free_context(&p); } };
struct InputFree { void operator()(AVFormatContext* p) const { avformat_close_input(&p); } };
struct OutputFree { void operator()(AVFormatContext* p) const { if (p->pb) avio_closep(&p->pb); avformat_free_context(p); } };
using Frame = std::unique_ptr<AVFrame, FrameFree>;
using Packet = std::unique_ptr<AVPacket, PacketFree>;
using Codec = std::unique_ptr<AVCodecContext, CodecFree>;
using Input = std::unique_ptr<AVFormatContext, InputFree>;
using Output = std::unique_ptr<AVFormatContext, OutputFree>;
static Frame frame() { Frame f(av_frame_alloc()); require(bool(f), "allocate frame"); return f; }
static Packet packet() { Packet p(av_packet_alloc()); require(bool(p), "allocate packet"); return p; }

static Codec videoEncoder(const char* name, const ExportConfig& config, int width, int height, AVRational fps) {
    auto codec = avcodec_find_encoder_by_name(name); require(codec != nullptr, std::string("Missing encoder: ") + name);
    Codec enc(avcodec_alloc_context3(codec)); require(bool(enc), "allocate encoder");
    enc->width = width; enc->height = height; enc->sw_pix_fmt = AV_PIX_FMT_NV12;
    enc->time_base = {fps.den, fps.num}; enc->framerate = fps; enc->sample_aspect_ratio = {1, 1};
    enc->bit_rate = config.bitrate;
#ifdef BSR_METAL
    enc->gop_size = 240;
#endif
    enc->qmin = -1; enc->qmax = -1;
    enc->color_range = AVCOL_RANGE_MPEG; enc->colorspace = AVCOL_SPC_BT709;
    enc->color_primaries = AVCOL_PRI_BT709; enc->color_trc = AVCOL_TRC_BT709; enc->chroma_sample_location = AVCHROMA_LOC_LEFT;
    enc->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    return enc;
}

static void applyEncoderOptions(AVCodecContext* enc, const ExportConfig& config) {
    for (size_t i = 0; i < config.option_count; ++i)
        check(av_opt_set(enc, config.options[i].name, config.options[i].value, AV_OPT_SEARCH_CHILDREN), config.options[i].name);
}

// Both links preserve audio/video order and bound memory. Failure wakes both
// producers and consumers; normal closure drains the remaining work.
struct Work { Frame frame; Packet audio; };
class WorkQueue {
    std::mutex mutex;
    std::condition_variable changed;
    std::deque<Work> items;
    bool closed = false;
public:
    bool push(Work value) {
        std::unique_lock lock(mutex); checkInterrupted();
        while (!closed && items.size() == FRAME_SLOTS) { changed.wait_for(lock, std::chrono::milliseconds(100)); checkInterrupted(); }
        if (closed) return false;
        items.push_back(std::move(value)); changed.notify_all(); return true;
    }
    bool pop(Work& value) {
        std::unique_lock lock(mutex); checkInterrupted();
        while (!closed && items.empty()) { changed.wait_for(lock, std::chrono::milliseconds(100)); checkInterrupted(); }
        if (items.empty()) return false;
        value = std::move(items.front()); items.pop_front(); changed.notify_all(); return true;
    }
    void close(bool discard = false) {
        std::lock_guard lock(mutex); closed = true; if (discard) items.clear(); changed.notify_all();
    }
};

// One definition for presentation orientation, SDR conversion and the video
// timeline. Its FFmpeg types are compiled separately for every selected SDK.
class Normalize {
    AVFilterGraph* graph = nullptr;
    AVFilterContext *input = nullptr, *output = nullptr;
    AVStream* stream;
    AVFormatContext* format;
    AVRational fps;
    int64_t origin;
public:
    Normalize(AVFormatContext* f, AVStream* s, AVRational rate, int64_t start) : stream(s), format(f), fps(rate), origin(start) {}
    ~Normalize() { avfilter_graph_free(&graph); }
    void push(AVFrame* frame) {
        if (!frame) { require(input != nullptr, "source contains no video"); check(av_buffersrc_add_frame_flags(input, nullptr, 0), "flush normalization"); return; }
        require(frame->best_effort_timestamp != AV_NOPTS_VALUE, "source has no timestamps");
        frame->pts = frame->best_effort_timestamp - origin;
        if (!graph) {
            require(frame->color_trc != AVCOL_TRC_SMPTE2084 && frame->color_trc != AVCOL_TRC_ARIB_STD_B67, "GPU export requires SDR input");
            graph = avfilter_graph_alloc(); require(graph != nullptr, "allocate normalization graph");
            auto sar = av_guess_sample_aspect_ratio(format, stream, frame); if (sar.num <= 0 || sar.den <= 0) sar = {1, 1};
            char args[256]; std::snprintf(args, sizeof(args), "video_size=%dx%d:pix_fmt=%d:time_base=%d/%d:pixel_aspect=%d/%d", frame->width, frame->height, frame->format, stream->time_base.num, stream->time_base.den, sar.num, sar.den);
            input = avfilter_graph_alloc_filter(graph, avfilter_get_by_name("buffer"), "source"); require(input != nullptr, "allocate video source");
            auto params = av_buffersrc_parameters_alloc(); require(params != nullptr, "allocate source parameters");
            params->color_space = frame->colorspace; params->color_range = frame->color_range;
            int result = av_buffersrc_parameters_set(input, params); av_free(params); check(result, "set source color");
            check(avfilter_init_str(input, args), "initialize video source");
            AVFilterContext* tail = input; unsigned serial = 0;
            auto add = [&](const char* name, const char* options = nullptr) {
                AVFilterContext* next = nullptr; std::string id = std::string(name) + std::to_string(serial++);
                check(avfilter_graph_create_filter(&next, avfilter_get_by_name(name), id.c_str(), options, nullptr, graph), name);
                check(avfilter_link(tail, 0, next, 0), "link normalization"); tail = next;
            };
            auto fm = av_frame_get_side_data(frame, AV_FRAME_DATA_DISPLAYMATRIX);
            auto sm = fm ? nullptr : av_packet_side_data_get(stream->codecpar->coded_side_data, stream->codecpar->nb_coded_side_data, AV_PKT_DATA_DISPLAYMATRIX);
            if (fm || sm) {
                int32_t matrix[9]; require((fm ? fm->size : sm->size) >= sizeof(matrix), "invalid display matrix");
                std::memcpy(matrix, fm ? fm->data : sm->data, sizeof(matrix)); double angle = -av_display_rotation_get(matrix);
                require(std::isfinite(angle), "invalid rotation"); int rotation = (static_cast<int>(std::lround(angle)) + 360) % 360;
                if (rotation == 90) add("transpose", matrix[3] > 0 ? "cclock_flip" : "clock");
                else if (rotation == 270) add("transpose", matrix[3] < 0 ? "clock_flip" : "cclock");
                else if (rotation == 180 || rotation == 0) {
                    if (rotation == 180 && matrix[0] < 0) add("hflip");
                    if (matrix[4] < 0) add("vflip");
                } else { auto degrees = std::to_string(rotation) + "*PI/180"; add("rotate", degrees.c_str()); }
            }
            add("scale", "1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1:out_color_matrix=bt709:out_range=tv");
            add("pad", "1920:1080:(ow-iw)/2:(oh-ih)/2"); add("format", "pix_fmts=nv12");
            auto rate = "fps=" + std::to_string(fps.num) + "/" + std::to_string(fps.den); add("fps", rate.c_str());
            check(avfilter_graph_create_filter(&output, avfilter_get_by_name("buffersink"), "output", nullptr, nullptr, graph), "create sink");
            check(avfilter_link(tail, 0, output, 0), "link sink"); check(avfilter_graph_config(graph, nullptr), "configure normalization");
            require(av_cmp_q(av_buffersink_get_time_base(output), av_inv_q(fps)) == 0, "normalization time base");
        }
        check(av_buffersrc_add_frame_flags(input, frame, AV_BUFFERSRC_FLAG_KEEP_REF), "push source frame");
    }
    bool pop(AVFrame* frame) {
        if (!output) return false;
        int result = av_buffersink_get_frame(output, frame);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return false;
        check(result, "receive normalized frame"); frame->time_base = av_buffersink_get_time_base(output);
        av_frame_remove_side_data(frame, AV_FRAME_DATA_DISPLAYMATRIX); return true;
    }
};

template<class Compositor> static void run(const ExportConfig& config) {
    require(std::isfinite(config.fps) && config.fps >= 1 && config.fps <= 240 && config.bitrate > 0, "invalid export FPS or bitrate");
    std::signal(SIGINT, interruptSignal); std::signal(SIGTERM, interruptSignal);
    av_log_set_level(AV_LOG_WARNING); const auto started = Clock::now();
    std::atomic<bool> failed{false};
    AVFormatContext* rawInput = avformat_alloc_context(); require(rawInput != nullptr, "allocate input");
    rawInput->interrupt_callback = {[](void* value) -> int { return interrupted.load(std::memory_order_relaxed) || static_cast<std::atomic<bool>*>(value)->load(); }, &failed};
    int opened = avformat_open_input(&rawInput, config.source, nullptr, nullptr); Input input(rawInput); check(opened, "open source");
    check(avformat_find_stream_info(input.get(), nullptr), "inspect source");
    int vi = av_find_best_stream(input.get(), AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0); check(vi, "find video");
    int ai = av_find_best_stream(input.get(), AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);
    auto stream = input->streams[vi];
    const auto videoTimeBase = stream->time_base;
    const auto audioTimeBase = ai >= 0 ? input->streams[ai]->time_base : AVRational{1, 1};
    int64_t origin = input->start_time != AV_NOPTS_VALUE ? av_rescale_q(input->start_time, {1, AV_TIME_BASE}, stream->time_base) : (stream->start_time == AV_NOPTS_VALUE ? 0 : stream->start_time);
    auto decoder = avcodec_find_decoder(stream->codecpar->codec_id); require(decoder != nullptr, "source decoder unavailable");
    Codec dec(avcodec_alloc_context3(decoder)); require(bool(dec), "allocate decoder");
    check(avcodec_parameters_to_context(dec.get(), stream->codecpar), "set decoder parameters"); dec->pkt_timebase = stream->time_base; dec->thread_count = 0;
    check(avcodec_open2(dec.get(), decoder, nullptr), "open software decoder");
    const auto rate = av_d2q(config.fps, 1000000);
    Compositor gpu(1920, 1080, config.scene); auto enc = gpu.encoder(config, rate);
    AVFormatContext* rawOutput = nullptr; check(avformat_alloc_output_context2(&rawOutput, nullptr, "mp4", config.output), "allocate MP4");
    Output output(rawOutput); auto video = avformat_new_stream(output.get(), nullptr); require(video != nullptr, "create video stream");
    check(avcodec_parameters_from_context(video->codecpar, enc.get()), "set output codec");
    video->codecpar->codec_tag = config.hevc ? MKTAG('h','v','c','1') : MKTAG('a','v','c','1'); video->time_base = enc->time_base;
    AVStream* audio = nullptr;
    if (ai >= 0) { audio = avformat_new_stream(output.get(), nullptr); require(audio != nullptr, "create audio stream"); check(avcodec_parameters_copy(audio->codecpar, input->streams[ai]->codecpar), "copy audio parameters"); audio->codecpar->codec_tag = 0; audio->time_base = input->streams[ai]->time_base; }
    check(avio_open(&output->pb, config.output, AVIO_FLAG_WRITE), "open output");
    AVDictionary* options = nullptr; av_dict_set(&options, "movflags", "+faststart", 0);
    int header = avformat_write_header(output.get(), &options); av_dict_free(&options); check(header, "write MP4 header");
    WorkQueue decodedQueue, encodedQueue;
    std::mutex failureMutex; std::string failure;
    // The Rust boundary reports text; do not carry C++ runtime exception
    // objects between threads (V8 embeds a separate C++ runtime on Windows).
    auto fail = [&](const std::exception& error) {
        { std::lock_guard lock(failureMutex); if (!failed.load()) { failure = error.what(); failed.store(true); } }
        decodedQueue.close(true); encodedQueue.close(true);
    };
    int64_t decodedFrames = 0, decodedAudio = 0, encodedFrames = 0, encodedAudio = 0, packets = 0, count = 0;
    std::thread decodeThread, encodeThread;
    try {
        decodeThread = std::thread([&] { BSR_POOL { try {
            Normalize normalization(input.get(), stream, rate, origin);
            auto decoded = frame(), cfr = frame(); auto sourcePacket = packet(); int64_t previous = AV_NOPTS_VALUE;
            auto enqueue = [&](AVFrame* value) {
                require(value->pts != AV_NOPTS_VALUE && (previous == AV_NOPTS_VALUE || value->pts == previous + 1), "non-contiguous output timestamps"); previous = value->pts;
                Frame retained(av_frame_clone(value)); require(bool(retained), "retain decoded frame");
                require(decodedQueue.push(Work{std::move(retained), {}}), "decode queue stopped"); ++decodedFrames;
            };
            auto drainNormalized = [&] {
                while (normalization.pop(cfr.get())) { enqueue(cfr.get()); av_frame_unref(cfr.get()); }
            };
            auto drainDecoder = [&] {
                while (true) {
                    int r = avcodec_receive_frame(dec.get(), decoded.get()); if (r == AVERROR(EAGAIN) || r == AVERROR_EOF) break; check(r, "receive decoded frame");
                    if (config.prepared) {
                        // The project filter graph already applied orientation, scale,
                        // frame rate and BT.709 limited-range conversion. Preserve every
                        // planned frame without normalizing or rotating it a second time.
                        require(decoded->width == 1920 && decoded->height == 1080 && decoded->format == AV_PIX_FMT_NV12, "invalid prepared project frame");
                        require(decoded->color_trc != AVCOL_TRC_SMPTE2084 && decoded->color_trc != AVCOL_TRC_ARIB_STD_B67, "GPU export requires SDR input");
                        decoded->pts = decodedFrames; decoded->time_base = av_inv_q(rate); decoded->sample_aspect_ratio = {1, 1};
                        decoded->color_range = AVCOL_RANGE_MPEG; decoded->colorspace = AVCOL_SPC_BT709;
                        av_frame_remove_side_data(decoded.get(), AV_FRAME_DATA_DISPLAYMATRIX);
                        enqueue(decoded.get());
                    } else { normalization.push(decoded.get()); drainNormalized(); }
                    av_frame_unref(decoded.get());
                }
            };
            while (!failed.load()) BSR_POOL {
                checkInterrupted(); int r = av_read_frame(input.get(), sourcePacket.get()); if (r == AVERROR_EOF) break; check(r, "read source packet");
                if (sourcePacket->stream_index == vi) {
                    r = avcodec_send_packet(dec.get(), sourcePacket.get()); if (r == AVERROR(EAGAIN)) { drainDecoder(); r = avcodec_send_packet(dec.get(), sourcePacket.get()); }
                    check(r, "send source packet"); drainDecoder();
                } else if (audio && sourcePacket->stream_index == ai) {
                    auto copied = packet(); av_packet_move_ref(copied.get(), sourcePacket.get());
                    require(decodedQueue.push(Work{{}, std::move(copied)}), "audio queue stopped"); ++decodedAudio;
                }
                av_packet_unref(sourcePacket.get());
            }
            if (!failed.load()) {
                check(avcodec_send_packet(dec.get(), nullptr), "flush decoder"); drainDecoder();
                if (!config.prepared) { normalization.push(nullptr); drainNormalized(); }
            }
            decodedQueue.close();
        } catch (const std::exception& error) { fail(error); } } });
        encodeThread = std::thread([&] { BSR_POOL { try {
            auto encoded = packet();
            auto drain = [&] {
                while (true) {
                    int r = avcodec_receive_packet(enc.get(), encoded.get()); if (r == AVERROR(EAGAIN) || r == AVERROR_EOF) break; check(r, "receive encoded packet");
                    encoded->duration = 1; av_packet_rescale_ts(encoded.get(), enc->time_base, video->time_base); encoded->stream_index = video->index;
                    check(av_interleaved_write_frame(output.get(), encoded.get()), "write video packet"); check(output->pb->error, "video output I/O"); ++packets;
                }
            };
            Work work;
            while (encodedQueue.pop(work)) BSR_POOL {
                require(!failed.load(), "encoding cancelled");
                if (work.audio) {
                    auto p = work.audio.get(); auto tb = audioTimeBase; int64_t start = av_rescale_q(origin, videoTimeBase, tb);
                    if (p->pts != AV_NOPTS_VALUE) p->pts -= start; if (p->dts != AV_NOPTS_VALUE) p->dts -= start;
                    av_packet_rescale_ts(p, tb, audio->time_base); p->stream_index = audio->index; p->pos = -1;
                    check(av_interleaved_write_frame(output.get(), p), "copy audio packet"); check(output->pb->error, "audio output I/O"); ++encodedAudio;
                } else {
                    int r = avcodec_send_frame(enc.get(), work.frame.get()); if (r == AVERROR(EAGAIN)) { drain(); r = avcodec_send_frame(enc.get(), work.frame.get()); }
                    check(r, "send GPU frame"); drain(); ++encodedFrames;
                }
                work = Work{};
            }
            require(!failed.load(), "encoder stopped"); checkInterrupted();
            check(avcodec_send_frame(enc.get(), nullptr), "flush encoder"); drain();
            check(av_write_trailer(output.get()), "write MP4 trailer"); check(avio_closep(&output->pb), "close MP4");
        } catch (const std::exception& error) { fail(error); } } });
        std::deque<size_t> pending;
        auto finish = [&] { auto f = gpu.take(pending.front()); require(encodedQueue.push(Work{std::move(f), {}}), "encode queue stopped"); pending.pop_front(); };
        Work work;
        while (decodedQueue.pop(work)) BSR_POOL {
            require(!failed.load(), "rendering cancelled");
            if (work.audio) { require(encodedQueue.push(std::move(work)), "audio queue stopped"); continue; }
            auto f = work.frame.get(); const auto index = count % FRAME_SLOTS;
            if (pending.size() == FRAME_SLOTS) finish();
            double time = f->pts * av_q2d(f->time_base); gpu.submit(index, f, time); pending.push_back(index); ++count;
            if (count % 60 == 0) bsr_gpu_progress(count, time, std::chrono::duration<double>(Clock::now() - started).count(), 0);
            work = Work{};
        }
        require(!failed.load(), "pipeline stopped"); checkInterrupted();
        while (!pending.empty()) finish();
        require(count > 0, "source contains no video"); encodedQueue.close();
    } catch (const std::exception& error) { fail(error); }
    if (decodeThread.joinable()) decodeThread.join(); if (encodeThread.joinable()) encodeThread.join();
    require(!failed.load(), failure);
    require(decodedFrames == count && encodedFrames == count && packets == count, "pipeline frame counts disagree");
    require(decodedAudio == encodedAudio, "audio packet count mismatch");
    bsr_gpu_progress(count, count / config.fps, std::chrono::duration<double>(Clock::now() - started).count(), 1);
}

template<class Compositor> static int probeCodecs(char* error, size_t capacity, int32_t* qualityMax) {
    try {
        Compositor gpu(320, 240, nullptr);
        auto cpu = frame(); cpu->format = AV_PIX_FMT_NV12; cpu->width = 320; cpu->height = 240;
        cpu->color_range = AVCOL_RANGE_MPEG; cpu->colorspace = AVCOL_SPC_BT709;
        check(av_frame_get_buffer(cpu.get(), 32), "allocate probe frame");
        for (int y = 0; y < 240; ++y) std::memset(cpu->data[0] + y * cpu->linesize[0], 16, 320);
        for (int y = 0; y < 120; ++y) std::memset(cpu->data[1] + y * cpu->linesize[1], 128, 320);
        int mask = 0; std::string failures;
        for (int hevc = 0; hevc < 2; ++hevc) try {
            ExportConfig config{nullptr, nullptr, nullptr, 60, 1000000, hevc, nullptr, 0, 0}; auto encoder = gpu.encoder(config, {60, 1});
            gpu.submit(0, cpu.get(), 0); auto f = gpu.take(0); f->pts = 0; f->duration = 1;
            check(avcodec_send_frame(encoder.get(), f.get()), "send GPU probe frame"); auto p = packet(); int count = 0, r;
            while ((r = avcodec_receive_packet(encoder.get(), p.get())) >= 0) { ++count; av_packet_unref(p.get()); }
            require(r == AVERROR(EAGAIN) || r == AVERROR_EOF, "receive probe packet");
            check(avcodec_send_frame(encoder.get(), nullptr), "flush GPU probe");
            while ((r = avcodec_receive_packet(encoder.get(), p.get())) >= 0) { ++count; av_packet_unref(p.get()); }
            require(r == AVERROR_EOF && count == 1, "probe did not encode one GPU frame");
            qualityMax[hevc] = gpu.qualityMax(encoder.get()); mask |= 1 << hevc;
        } catch (const std::exception& e) { failures += e.what(); failures += '\n'; }
        require(mask != 0, failures); return mask;
    } catch (const std::exception& e) { std::snprintf(error, capacity, "%s", e.what()); return -1; }
}

template<class Compositor> static int exportVideo(const ExportConfig* config, char* error, size_t capacity) {
    try { run<Compositor>(*config); return 0; }
    catch (const std::exception& e) { std::snprintf(error, capacity, "%s", e.what()); return interrupted.load() ? 128 + interrupted.load() : 1; }
}
} // namespace
