#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <CoreVideo/CoreVideo.h>
#include "skia_bridge.h"
#include "shader.h"
extern "C" {
#include <libavformat/avformat.h>
#include <libavcodec/avcodec.h>
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersrc.h>
#include <libavfilter/buffersink.h>
#include <libavutil/display.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_videotoolbox.h>
#include <libavutil/opt.h>
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
#include <pthread.h>
#include <string>
#include <thread>

#include "ffmpeg_api.h"

namespace {
using Clock = std::chrono::steady_clock;
struct PipelineError { std::string message; };
static double seconds(Clock::time_point t) {
    return std::chrono::duration<double>(Clock::now() - t).count();
}
static void require(bool value, const std::string& message) {
    if (!value) throw PipelineError{message};
}
static void check(int value, const char* message) {
    if (value < 0) {
        char text[256];
        av_strerror(value, text, sizeof(text));
        throw PipelineError{std::string(message) + ": " + text};
    }
}
static std::string str(NSString* text) { return text ? text.UTF8String : ""; }
static void progress(NSDictionary* value) {
    NSData* data = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    fwrite(data.bytes, 1, data.length, stdout);
    fputc('\n', stdout);
    fflush(stdout);
}
static void releasePixel(void*, uint8_t* pixel) {
    CVPixelBufferRelease(reinterpret_cast<CVPixelBufferRef>(pixel));
}

static std::atomic<int> interrupted{0};
static_assert(std::atomic<int>::is_always_lock_free);
static void interruptSignal(int value) { interrupted.store(value, std::memory_order_relaxed); }
static void checkInterrupted() { require(!interrupted.load(std::memory_order_relaxed), "cancelled"); }
struct FrameFree { void operator()(AVFrame* value) const { av_frame_free(&value); } };
struct PacketFree { void operator()(AVPacket* value) const { av_packet_free(&value); } };
using Frame = std::unique_ptr<AVFrame, FrameFree>;
using Packet = std::unique_ptr<AVPacket, PacketFree>;
struct Work { Frame frame; Packet audio; };

// Both links carry ordered video frames and audio packets. Closing drains queued
// work; aborting drops it and wakes both producer and consumer on any failure.
class WorkQueue {
    std::mutex mutex;
    std::condition_variable changed;
    std::deque<Work> items;
    bool closed = false;
public:
    static constexpr size_t capacity = 3;
    bool push(Work value) {
        std::unique_lock lock(mutex);
        checkInterrupted();
        while (!closed && items.size() == capacity) {
            changed.wait_for(lock, std::chrono::milliseconds(100));
            checkInterrupted();
        }
        if (closed) return false;
        items.push_back(std::move(value));
        changed.notify_all();
        return true;
    }
    bool pop(Work& value) {
        std::unique_lock lock(mutex);
        checkInterrupted();
        while (!closed && items.empty()) {
            changed.wait_for(lock, std::chrono::milliseconds(100));
            checkInterrupted();
        }
        if (items.empty()) return false;
        value = std::move(items.front());
        items.pop_front();
        changed.notify_all();
        return true;
    }
    void close(bool discard = false) {
        std::lock_guard lock(mutex);
        closed = true;
        if (discard) items.clear();
        changed.notify_all();
    }
};

struct FrameSlot {
    __strong id<MTLTexture> overlay;
    __strong id<MTLCommandBuffer> command;
    CVMetalTextureRef views[4] = {};
    AVFrame* frame = nullptr;
    CVPixelBufferRef uploadedSource = nullptr;
};

class Compositor {
public:
    int width, height;
    __strong id<MTLDevice> device;
    __strong id<MTLCommandQueue> queue;
    __strong id<MTLComputePipelineState> pipeline;
    CVMetalTextureCacheRef textureCache = nullptr;
    CVPixelBufferPoolRef pool = nullptr;
    std::array<FrameSlot, 3> slots;

    Compositor(int w, int h) : width(w), height(h) {
        device = MTLCreateSystemDefaultDevice();
        require(device != nil, "Metal device");
        queue = [device newCommandQueue];
        require(queue != nil, "Metal queue");
        NSError* error = nil;
        NSString* source = [NSString stringWithUTF8String:COMPOSITE_SHADER];
        require(source != nil, "read shader: " + str(error.localizedDescription));
        MTLCompileOptions* options = [MTLCompileOptions new];
        options.languageVersion = MTLLanguageVersion2_4;
        id<MTLLibrary> library = [device newLibraryWithSource:source options:options error:&error];
        require(library != nil, "compile shader: " + str(error.localizedDescription));
        pipeline = [device newComputePipelineStateWithFunction:[library newFunctionWithName:@"composite_nv12"] error:&error];
        require(pipeline != nil, "compute pipeline: " + str(error.localizedDescription));
        require(CVMetalTextureCacheCreate(nullptr, nullptr, device, nullptr, &textureCache) == kCVReturnSuccess,
                "CVMetalTextureCacheCreate");
        NSDictionary* attributes = @{
            (id)kCVPixelBufferWidthKey: @(w), (id)kCVPixelBufferHeightKey: @(h),
            (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange),
            (id)kCVPixelBufferMetalCompatibilityKey: @YES,
            (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
        };
        require(CVPixelBufferPoolCreate(nullptr, nullptr, (__bridge CFDictionaryRef)attributes, &pool) == kCVReturnSuccess,
                "CVPixelBufferPoolCreate");
        auto descriptor = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                                          width:w height:h mipmapped:NO];
        descriptor.storageMode = MTLStorageModePrivate;
        descriptor.usage = MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead;
        for (auto& slot : slots) {
            slot.overlay = [device newTextureWithDescriptor:descriptor];
            require(slot.overlay != nil, "Skia render texture");
        }
    }
    ~Compositor() {
        for (auto& slot : slots) {
            if (slot.command) [slot.command waitUntilCompleted];
            release(slot);
        }
        if (pool) CVPixelBufferPoolRelease(pool);
        if (textureCache) CFRelease(textureCache);
    }
    void release(FrameSlot& slot) {
        av_frame_free(&slot.frame);
        for (auto& view : slot.views) {
            if (view) { CFRelease(view); view = nullptr; }
        }
        if(slot.uploadedSource){CVPixelBufferRelease(slot.uploadedSource);slot.uploadedSource=nullptr;}
        slot.command = nil;
    }
    CVPixelBufferRef upload(FrameSlot& slot, const AVFrame* frame) {
        require(frame->format==AV_PIX_FMT_NV12,"normalization must produce NV12");
        require(CVPixelBufferPoolCreatePixelBuffer(nullptr,pool,&slot.uploadedSource)==kCVReturnSuccess,"allocate software upload buffer");
        auto pixel=slot.uploadedSource;
        require(CVPixelBufferLockBaseAddress(pixel,0)==kCVReturnSuccess,"lock software upload buffer");
        auto y=(uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pixel,0);
        auto uv=(uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pixel,1);
        size_t ys=CVPixelBufferGetBytesPerRowOfPlane(pixel,0),uvs=CVPixelBufferGetBytesPerRowOfPlane(pixel,1);
        for(int row=0;row<height;row++)std::memcpy(y+row*ys,frame->data[0]+row*frame->linesize[0],width);
        for(int row=0;row<height/2;row++)std::memcpy(uv+row*uvs,frame->data[1]+row*frame->linesize[1],width);
        require(CVPixelBufferUnlockBaseAddress(pixel,0)==kCVReturnSuccess,"unlock software upload buffer");
        return pixel;
    }
    void wait(FrameSlot& slot) {
        if (slot.command.status != MTLCommandBufferStatusCompleted) {
            [slot.command waitUntilCompleted];
        }
        require(slot.command.status == MTLCommandBufferStatusCompleted,
                "GPU command: " + str(slot.command.error.localizedDescription));
    }
    CVMetalTextureRef view(CVPixelBufferRef pixel, int plane, bool destination) {
        CVMetalTextureRef result = nullptr;
        auto usage = destination ? MTLTextureUsageShaderWrite : MTLTextureUsageShaderRead;
        NSDictionary* attributes = @{(id)kCVMetalTextureUsage: @(usage)};
        CVReturn status = CVMetalTextureCacheCreateTextureFromImage(
            nullptr, textureCache, pixel, (__bridge CFDictionaryRef)attributes,
            plane ? MTLPixelFormatRG8Unorm : MTLPixelFormatR8Unorm,
            width >> plane, height >> plane, plane, &result);
        require(status == kCVReturnSuccess, "CVMetalTexture view: " + std::to_string(status));
        return result;
    }
    CVPixelBufferRef compose(FrameSlot& slot, CVPixelBufferRef source) {
        require(CVPixelBufferGetPixelFormatType(source) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                "source is not video-range NV12");
        CVPixelBufferRef output = nullptr;
        require(CVPixelBufferPoolCreatePixelBuffer(nullptr, pool, &output) == kCVReturnSuccess,
                "allocate output frame");
        // Skia flushed onto this same command queue. The compute dispatch follows
        // its writes without a CPU readback or a per-frame completion wait.
        slot.command = [queue commandBuffer];
        for (int plane = 0; plane < 2; ++plane) {
            slot.views[plane] = view(source, plane, false);
            slot.views[plane + 2] = view(output, plane, true);
        }
        auto encoder = [slot.command computeCommandEncoder];
        [encoder setComputePipelineState:pipeline];
        for (int i = 0; i < 2; ++i)
            [encoder setTexture:CVMetalTextureGetTexture(slot.views[i]) atIndex:i];
        [encoder setTexture:slot.overlay atIndex:2];
        for (int i = 0; i < 2; ++i)
            [encoder setTexture:CVMetalTextureGetTexture(slot.views[i + 2]) atIndex:i + 3];
        [encoder dispatchThreads:MTLSizeMake(width / 2, height / 2, 1)
           threadsPerThreadgroup:MTLSizeMake(16, 16, 1)];
        [encoder endEncoding];
        [slot.command commit];
        return output;
    }
};

static int runExport(const char* path) {
 @autoreleasepool { try {
    std::signal(SIGINT, interruptSignal); std::signal(SIGTERM, interruptSignal);
    pthread_setname_np("bsr.render");
    auto started=Clock::now();
    pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0);
    av_log_set_level(AV_LOG_WARNING);
    NSData *data=[NSData dataWithContentsOfFile:[NSString stringWithUTF8String:path]];
    NSDictionary *config=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    require(config!=nil,"read config");
    std::string source=str(config[@"source"]),output=str(config[@"output"]);
    AVFormatContext *in=nullptr; check(avformat_open_input(&in,source.c_str(),nullptr,nullptr),"open source");
    check(avformat_find_stream_info(in,nullptr),"source streams");
    int vi=av_find_best_stream(in,AVMEDIA_TYPE_VIDEO,-1,-1,nullptr,0); check(vi,"video stream");
    int ai=av_find_best_stream(in,AVMEDIA_TYPE_AUDIO,-1,-1,nullptr,0);
    auto stream=in->streams[vi]; const int w=1920,h=1080;
    const int64_t origin = in->start_time != AV_NOPTS_VALUE ?
        av_rescale_q(in->start_time, {1, AV_TIME_BASE}, stream->time_base) :
        (stream->start_time != AV_NOPTS_VALUE ? stream->start_time : 0);
    double rate = [config[@"fps"] doubleValue];
    require(std::isfinite(rate) && rate >= 60 && rate <= 240, "output FPS between 60 and 240");
    const AVRational frameRate = av_d2q(rate, 1000000);
    const AVRational frameTimeBase = {frameRate.den, frameRate.num};
    AVBufferRef *device=nullptr; check(av_hwdevice_ctx_create(&device,AV_HWDEVICE_TYPE_VIDEOTOOLBOX,nullptr,nullptr,0),"VideoToolbox device");
    auto decoder=avcodec_find_decoder(stream->codecpar->codec_id);
    auto dec=avcodec_alloc_context3(decoder); check(avcodec_parameters_to_context(dec,stream->codecpar),"decoder params");
    dec->thread_count=0;
    check(avcodec_open2(dec,decoder,nullptr),"open software decoder");

    Compositor compositor(w,h);
    std::unique_ptr<void, decltype(&skia_renderer_free)> renderer(
        skia_renderer_new([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String, (__bridge void *)compositor.device,
                          (__bridge void *)compositor.queue, (__bridge void *)compositor.slots[0].overlay),
        skia_renderer_free);
    require(renderer != nullptr,"initialize Skia Metal renderer");

    AVBufferRef *encodeFrames=av_hwframe_ctx_alloc(device);
    auto ef=(AVHWFramesContext *)encodeFrames->data;
    ef->format=AV_PIX_FMT_VIDEOTOOLBOX;ef->sw_format=AV_PIX_FMT_NV12;ef->width=w;ef->height=h;
    ((AVVTFramesContext *)ef->hwctx)->color_range=AVCOL_RANGE_MPEG;
    check(av_hwframe_ctx_init(encodeFrames),"encoder hardware frames");
    AVFormatContext *mux=nullptr; AVCodecContext *enc=nullptr; AVStream *videoOut=nullptr,*audioOut=nullptr;
    {
        check(avformat_alloc_output_context2(&mux,nullptr,"mp4",output.c_str()),"MP4 muxer");
        std::string encoderName=str(config[@"encoder"]);
        require(encoderName=="hevc_videotoolbox"||encoderName=="h264_videotoolbox","VideoToolbox H.264 or HEVC encoder");
        auto codec=avcodec_find_encoder_by_name(encoderName.c_str()); require(codec!=nullptr,"VideoToolbox encoder");
        enc=avcodec_alloc_context3(codec); enc->width=w;enc->height=h;enc->pix_fmt=AV_PIX_FMT_VIDEOTOOLBOX;
        enc->sw_pix_fmt=AV_PIX_FMT_NV12;enc->hw_frames_ctx=av_buffer_ref(encodeFrames);
        enc->time_base=frameTimeBase;enc->framerate=frameRate;enc->sample_aspect_ratio={1,1};
        enc->bit_rate=[config[@"bitrate_bps"] longLongValue];require(enc->bit_rate>0,"positive target bitrate");
        enc->gop_size=120;enc->max_b_frames=0;enc->qmin=-1;enc->qmax=-1;
        enc->color_range=AVCOL_RANGE_MPEG;enc->colorspace=AVCOL_SPC_BT709;enc->color_primaries=AVCOL_PRI_BT709;enc->color_trc=AVCOL_TRC_BT709;enc->chroma_sample_location=AVCHROMA_LOC_LEFT;
        if(mux->oformat->flags&AVFMT_GLOBALHEADER) enc->flags|=AV_CODEC_FLAG_GLOBAL_HEADER;
        av_opt_set(enc->priv_data,"allow_sw","0",0);
        check(av_opt_set(enc->priv_data,"prio_speed",[config[@"speed_priority"] boolValue]?"1":"0",0),"encoding speed priority");
        check(avcodec_open2(enc,codec,nullptr),"open VideoToolbox hardware encoder");
        videoOut=avformat_new_stream(mux,nullptr); check(avcodec_parameters_from_context(videoOut->codecpar,enc),"video output params");
        videoOut->codecpar->codec_tag=codec->id==AV_CODEC_ID_H264?MKTAG('a','v','c','1'):MKTAG('h','v','c','1');videoOut->time_base=enc->time_base;
        if(ai>=0){audioOut=avformat_new_stream(mux,nullptr);check(avcodec_parameters_copy(audioOut->codecpar,in->streams[ai]->codecpar),"audio copy params");audioOut->codecpar->codec_tag=0;audioOut->time_base=in->streams[ai]->time_base;}
        check(avio_open(&mux->pb,output.c_str(),AVIO_FLAG_WRITE),"open MP4");
        AVDictionary *opts=nullptr;av_dict_set(&opts,"movflags","+faststart",0);check(avformat_write_header(mux,&opts),"MP4 header");av_dict_free(&opts);
    }
    const AVRational videoTimeBase=stream->time_base;
    const AVRational audioTimeBase=ai>=0?in->streams[ai]->time_base:AVRational{1,1};
    WorkQueue decodedQueue,encodedQueue;
    std::atomic<bool> failed{false};
    std::mutex failureMutex;std::exception_ptr failure;
    auto fail=[&](std::exception_ptr error){
        {std::lock_guard lock(failureMutex);if(!failure)failure=error;}
        failed.store(true);
        decodedQueue.close(true);encodedQueue.close(true);
    };
    in->interrupt_callback={[](void* value)->int{
        return interrupted.load(std::memory_order_relaxed)||static_cast<std::atomic<bool>*>(value)->load();
    },&failed};
    int64_t decodedFrames=0,decodedAudioPackets=0;
    int64_t encodedFrames=0,encodedVideoPackets=0,encodedAudioPackets=0;
    int64_t count=0;
    std::thread decodeThread,encodeThread;
    std::deque<size_t> pending;
    try {
        decodeThread=std::thread([&]{@autoreleasepool {
            pthread_setname_np("bsr.decode");
            AVFilterGraph* graph=nullptr;AVFilterContext *buffer=nullptr,*sink=nullptr;
            try {
                Packet packet(av_packet_alloc());Frame decoded(av_frame_alloc()),cfr(av_frame_alloc());
                require(packet&&decoded&&cfr,"allocate decode work");
                int64_t previousPTS=AV_NOPTS_VALUE;
                auto drainFPS=[&]{
                    while(true){
                        checkInterrupted();
                        int ret=av_buffersink_get_frame(sink,cfr.get());
                        if(ret==AVERROR(EAGAIN)||ret==AVERROR_EOF)break;
                        check(ret,"receive fps frame");
                        {
                            require(cfr->pts!=AV_NOPTS_VALUE && (previousPTS==AV_NOPTS_VALUE || cfr->pts==previousPTS+1),"decoded frame sequence");
                            previousPTS=cfr->pts;
                            cfr->time_base=av_buffersink_get_time_base(sink);
                            // Orientation is baked into pixels before danmaku composition.
                            av_frame_remove_side_data(cfr.get(),AV_FRAME_DATA_DISPLAYMATRIX);
                            Frame frame(av_frame_clone(cfr.get()));require(frame!=nullptr,"retain decoded frame");
                            require(decodedQueue.push(Work{std::move(frame),{}}),"decode queue stopped");
                            ++decodedFrames;
                        }
                        av_frame_unref(cfr.get());
                    }
                };
                auto drainDecoder=[&]{
                    while(true){
                        checkInterrupted();
                        int ret=avcodec_receive_frame(dec,decoded.get());
                        if(ret==AVERROR(EAGAIN)||ret==AVERROR_EOF)break;
                        check(ret,"receive decoded frame");
                        require(decoded->best_effort_timestamp != AV_NOPTS_VALUE,"source frame has no timestamp");
                        decoded->pts=decoded->best_effort_timestamp-origin;
                        if(!graph){
                            require(decoded->color_trc!=AVCOL_TRC_SMPTE2084 && decoded->color_trc!=AVCOL_TRC_ARIB_STD_B67,
                                    "Metal export requires SDR input; convert HDR to SDR before burning danmaku");
                            graph=avfilter_graph_alloc();require(graph!=nullptr,"allocate fps graph");
                            auto sar=av_guess_sample_aspect_ratio(in,stream,decoded.get());
                            if(sar.num<=0||sar.den<=0)sar={1,1};
                            char args[256];snprintf(args,sizeof(args),"video_size=%dx%d:pix_fmt=%d:time_base=%d/%d:pixel_aspect=%d/%d",decoded->width,decoded->height,decoded->format,videoTimeBase.num,videoTimeBase.den,sar.num,sar.den);
                            buffer=avfilter_graph_alloc_filter(graph,avfilter_get_by_name("buffer"),"input");require(buffer!=nullptr,"allocate fps source");
                            auto params=av_buffersrc_parameters_alloc();require(params!=nullptr,"fps source parameters");
                            params->hw_frames_ctx=decoded->hw_frames_ctx;params->color_space=decoded->colorspace;params->color_range=decoded->color_range;
                            int ret=av_buffersrc_parameters_set(buffer,params);av_free(params);check(ret,"hardware fps source");
                            check(avfilter_init_str(buffer,args),"fps buffer source");
                            // Apply presentation orientation before fitting the video to the canvas.
                            AVFilterContext* oriented=buffer;
                            auto frameMatrix=av_frame_get_side_data(decoded.get(),AV_FRAME_DATA_DISPLAYMATRIX);
                            auto streamMatrix=frameMatrix?nullptr:av_packet_side_data_get(stream->codecpar->coded_side_data,
                                stream->codecpar->nb_coded_side_data,AV_PKT_DATA_DISPLAYMATRIX);
                            if(frameMatrix||streamMatrix){
                                int32_t matrix[9];
                                require((frameMatrix?frameMatrix->size:streamMatrix->size)>=sizeof(matrix),"invalid display matrix");
                                std::memcpy(matrix,frameMatrix?frameMatrix->data:streamMatrix->data,sizeof(matrix));
                                double angle=-av_display_rotation_get(matrix);
                                require(std::isfinite(angle),"invalid display rotation");
                                int rotation=(static_cast<int>(std::lround(angle))+360)%360;
                                auto transform=[&](const char* name,const char* options=nullptr){
                                    AVFilterContext* next=nullptr;
                                    check(avfilter_graph_create_filter(&next,avfilter_get_by_name(name),name,options,nullptr,graph),"orientation filter");
                                    check(avfilter_link(oriented,0,next,0),"link orientation");
                                    oriented=next;
                                };
                                switch(rotation){
                                    case 90: transform("transpose",matrix[3]>0?"cclock_flip":"clock");break;
                                    case 180:
                                        if(matrix[0]<0)transform("hflip");
                                        if(matrix[4]<0)transform("vflip");
                                        break;
                                    case 270: transform("transpose",matrix[3]<0?"clock_flip":"cclock");break;
                                    case 0:
                                        if(matrix[4]<0)transform("vflip");
                                        break;
                                    default: {
                                        char degrees[64];snprintf(degrees,sizeof(degrees),"%d*PI/180",rotation);
                                        transform("rotate",degrees);
                                        break;
                                    }
                                }
                            }
                            AVFilterContext *scale=nullptr,*pad=nullptr,*format=nullptr,*fps=nullptr;
                            check(avfilter_graph_create_filter(&scale,avfilter_get_by_name("scale"),"scale",
                                "1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1:out_color_matrix=bt709:out_range=tv",nullptr,graph),"scale filter");
                            check(avfilter_graph_create_filter(&pad,avfilter_get_by_name("pad"),"pad","1920:1080:(ow-iw)/2:(oh-ih)/2",nullptr,graph),"pad filter");
                            check(avfilter_graph_create_filter(&format,avfilter_get_by_name("format"),"format","pix_fmts=nv12",nullptr,graph),"format filter");
                            std::string fpsArgs="fps="+std::to_string(frameRate.num)+"/"+std::to_string(frameRate.den);
                            check(avfilter_graph_create_filter(&fps,avfilter_get_by_name("fps"),"fps",fpsArgs.c_str(),nullptr,graph),"fps filter");
                            check(avfilter_graph_create_filter(&sink,avfilter_get_by_name("buffersink"),"output",nullptr,nullptr,graph),"fps sink");
                            check(avfilter_link(oriented,0,scale,0),"link scale");check(avfilter_link(scale,0,pad,0),"link pad");
                            check(avfilter_link(pad,0,format,0),"link format");check(avfilter_link(format,0,fps,0),"link fps");
                            check(avfilter_link(fps,0,sink,0),"link sink");check(avfilter_graph_config(graph,nullptr),"configure normalization");
                            require(av_cmp_q(av_buffersink_get_time_base(sink),frameTimeBase)==0,"fps time base");
                        }
                        check(av_buffersrc_add_frame_flags(buffer,decoded.get(),AV_BUFFERSRC_FLAG_KEEP_REF),"push decoded frame");
                        av_frame_unref(decoded.get());drainFPS();
                    }
                };
                while(!failed.load()){@autoreleasepool {
                    checkInterrupted();
                    int ret=av_read_frame(in,packet.get());
                    if(ret==AVERROR_EOF)break;
                    check(ret,"read source packet");
                    if(packet->stream_index==vi){
                        check(avcodec_send_packet(dec,packet.get()),"send source packet");
                        drainDecoder();
                    }else if(audioOut&&packet->stream_index==ai){
                        Packet audio(av_packet_alloc());require(audio!=nullptr,"allocate audio packet");
                        av_packet_move_ref(audio.get(),packet.get());
                        require(decodedQueue.push(Work{{},std::move(audio)}),"audio queue stopped");
                        ++decodedAudioPackets;
                    }
                    av_packet_unref(packet.get());
                }}
                if(!failed.load()){
                    check(avcodec_send_packet(dec,nullptr),"flush decoder");
                    drainDecoder();
                    require(buffer!=nullptr,"no decoded video");
                    check(av_buffersrc_add_frame_flags(buffer,nullptr,0),"flush fps");
                    drainFPS();
                }
                decodedQueue.close();
            }catch(...){fail(std::current_exception());}
            avfilter_graph_free(&graph);
        }});
        encodeThread=std::thread([&]{@autoreleasepool {
            pthread_setname_np("bsr.encode");
            try {
                Packet packet(av_packet_alloc());require(packet!=nullptr,"allocate encoded packet");
                auto drainEncoder=[&](){
                    while(true){
                        int ret=avcodec_receive_packet(enc,packet.get());
                        if(ret==AVERROR(EAGAIN)||ret==AVERROR_EOF)break;
                        check(ret,"receive encoded packet");
                        packet->duration=1;av_packet_rescale_ts(packet.get(),enc->time_base,videoOut->time_base);packet->stream_index=videoOut->index;
                        check(av_interleaved_write_frame(mux,packet.get()),"write video packet");
                        check(mux->pb->error,"video output I/O");av_packet_unref(packet.get());++encodedVideoPackets;
                    }
                };
                Work work;
                while(encodedQueue.pop(work)){@autoreleasepool {
                    require(!failed.load(),"encoding cancelled");
                    if(work.audio){
                        auto p=work.audio.get();int64_t a0=av_rescale_q(origin,videoTimeBase,audioTimeBase);
                        if(p->pts!=AV_NOPTS_VALUE)p->pts-=a0;if(p->dts!=AV_NOPTS_VALUE)p->dts-=a0;av_packet_rescale_ts(p,audioTimeBase,audioOut->time_base);p->stream_index=audioOut->index;p->pos=-1;
                        check(av_interleaved_write_frame(mux,p),"copy audio packet");
                        check(mux->pb->error,"audio output I/O");++encodedAudioPackets;
                    }else{
                        require(work.frame!=nullptr,"encode frame");
                        check(avcodec_send_frame(enc,work.frame.get()),"send hardware frame");
                        drainEncoder();
                        ++encodedFrames;
                    }
                    work=Work{};
                }}
                require(!failed.load(),"encoder stopped");checkInterrupted();
                {
                    check(avcodec_send_frame(enc,nullptr),"flush encoder");drainEncoder();
                    check(av_write_trailer(mux),"MP4 trailer");
                    check(avio_closep(&mux->pb),"close MP4");
                }
            }catch(...){fail(std::current_exception());}
        }});
        auto finishFrame=[&]{
            auto& slot=compositor.slots[pending.front()];compositor.wait(slot);
            Frame frame(slot.frame);slot.frame=nullptr;
            require(encodedQueue.push(Work{std::move(frame),{}}),"encode queue stopped");
            compositor.release(slot);pending.pop_front();
        };
        Work work;
        while(decodedQueue.pop(work)){@autoreleasepool {
            require(!failed.load(),"rendering cancelled");
            if(work.audio){
                require(encodedQueue.push(std::move(work)),"audio output queue stopped");
                continue;
            }
            auto cfr=work.frame.get();require(cfr!=nullptr,"render frame");
            size_t index=count%compositor.slots.size();
            if(pending.size()==compositor.slots.size()){
                require(pending.front()==index,"frame slot order");finishFrame();
            }
            auto& slot=compositor.slots[index];
            double now=cfr->pts*av_q2d(cfr->time_base);
            require(skia_renderer_draw(renderer.get(),now,(__bridge void*)slot.overlay)==0,"draw Skia Metal frame");
            CVPixelBufferRef input=compositor.upload(slot,cfr);
            CVPixelBufferRef pb=compositor.compose(slot,input);
            AVFrame* f=av_frame_alloc();require(f!=nullptr,"allocate output frame");slot.frame=f;
            f->format=AV_PIX_FMT_VIDEOTOOLBOX;f->width=w;f->height=h;f->hw_frames_ctx=av_buffer_ref(encodeFrames);
            f->data[3]=(uint8_t*)pb;f->buf[0]=av_buffer_create((uint8_t*)pb,1,releasePixel,nullptr,0);
            if(!f->buf[0]){CVPixelBufferRelease(pb);throw PipelineError{"pixel buffer owner"};}
            check(av_frame_copy_props(f,cfr),"frame metadata");f->pts=cfr->pts;f->duration=1;f->time_base=frameTimeBase;
            // Input I-frames describe the source bitstream, not output keyframe requests.
            f->pict_type=AV_PICTURE_TYPE_NONE;f->flags&=~AV_FRAME_FLAG_KEY;
            check(av_vt_pixbuf_set_attachments(nullptr,pb,f),"pixel buffer metadata");
            pending.push_back(index);++count;
            if(count%60==0)progress(@{@"event":@"progress",@"frames":@(count),@"time_seconds":@(now),@"elapsed_seconds":@(seconds(started))});
        }}
        require(!failed.load(),"pipeline stopped");checkInterrupted();
        while(!pending.empty())finishFrame();
        require(count>0,"source contains no video frames");
        encodedQueue.close();
    }catch(...){
        fail(std::current_exception());
        // Finish submitted Metal work while its Skia context is still alive.
        for(auto& slot:compositor.slots){if(slot.command)[slot.command waitUntilCompleted];compositor.release(slot);}
    }
    if(decodeThread.joinable())decodeThread.join();if(encodeThread.joinable())encodeThread.join();
    if(failure)std::rethrow_exception(failure);
    require(decodedFrames==count&&encodedFrames==count,"pipeline frame counts disagree");
    require(encodedVideoPackets==count,"encoded packet count");
    require(decodedAudioPackets==encodedAudioPackets,"audio packet count");
    avcodec_free_context(&enc);if(mux)avformat_free_context(mux);avcodec_free_context(&dec);avformat_close_input(&in);av_buffer_unref(&device);av_buffer_unref(&encodeFrames);
    progress(@{@"ok":@YES,@"frames":@(count),@"duration_seconds":@(count/av_q2d(frameRate)),@"total_seconds":@(seconds(started))});
    return 0;
 }catch(const PipelineError& error){fprintf(stderr,"ERROR: %s\n",error.message.c_str());return interrupted.load()?128+interrupted.load():1;}
 catch(...){fprintf(stderr,"ERROR: unexpected native exception\n");return interrupted.load()?128+interrupted.load():1;} }
}

// Check hardware-session availability using the selected FFmpeg adapter.
static int probeCodecs() {
 @autoreleasepool {
    if (!MTLCreateSystemDefaultDevice()) return 0;
    int mask=0;
    const char* names[]={"h264_videotoolbox","hevc_videotoolbox"};
    for(int i=0;i<2;i++) {
        const AVCodec* codec=avcodec_find_encoder_by_name(names[i]);
        if(!codec)continue;
        AVCodecContext* enc=avcodec_alloc_context3(codec);
        if(!enc)continue;
        enc->width=320;enc->height=240;enc->pix_fmt=AV_PIX_FMT_NV12;
        enc->time_base={1,60};enc->framerate={60,1};enc->bit_rate=1000000;
        enc->gop_size=120;enc->max_b_frames=0;enc->qmin=-1;enc->qmax=-1;
        av_opt_set(enc->priv_data,"allow_sw","0",0);
        if(avcodec_open2(enc,codec,nullptr)>=0)mask|=1<<i;
        avcodec_free_context(&enc);
    }
    return mask;
 }
}
} // namespace

extern "C" const char* bsr_ffmpeg_load(const char* directory) { return loadFFmpeg(directory); }
extern "C" int bsr_metal_export(const char* path) { return runExport(path); }
extern "C" int bsr_videotoolbox_probe() { return probeCodecs(); }
