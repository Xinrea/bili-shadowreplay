#pragma once

#include <dlfcn.h>

namespace {
// These declarations and their callers live in one anonymous namespace per
// FFmpeg ABI. A pointer always has the type from that ABI's public headers.
#define FFMPEG_API(X) \
    X(0, av_buffer_create) \
    X(0, av_buffer_ref) \
    X(0, av_buffer_unref) \
    X(0, av_d2q) \
    X(0, av_dict_free) \
    X(0, av_dict_set) \
    X(0, av_display_rotation_get) \
    X(0, av_frame_alloc) \
    X(0, av_frame_clone) \
    X(0, av_frame_copy_props) \
    X(0, av_frame_free) \
    X(0, av_frame_get_side_data) \
    X(0, av_frame_remove_side_data) \
    X(0, av_frame_unref) \
    X(0, av_free) \
    X(0, av_hwdevice_ctx_create) \
    X(0, av_hwframe_ctx_alloc) \
    X(0, av_hwframe_ctx_init) \
    X(0, av_log_set_level) \
    X(0, av_opt_set) \
    X(0, av_rescale_q) \
    X(0, av_strerror) \
    X(0, av_vt_pixbuf_set_attachments) \
    X(0, av_version_info) \
    X(0, avutil_version) \
    X(1, av_packet_alloc) \
    X(1, av_packet_free) \
    X(1, av_packet_move_ref) \
    X(1, av_packet_rescale_ts) \
    X(1, av_packet_side_data_get) \
    X(1, av_packet_unref) \
    X(1, avcodec_alloc_context3) \
    X(1, avcodec_find_decoder) \
    X(1, avcodec_find_encoder_by_name) \
    X(1, avcodec_free_context) \
    X(1, avcodec_open2) \
    X(1, avcodec_parameters_copy) \
    X(1, avcodec_parameters_from_context) \
    X(1, avcodec_parameters_to_context) \
    X(1, avcodec_receive_frame) \
    X(1, avcodec_receive_packet) \
    X(1, avcodec_send_frame) \
    X(1, avcodec_send_packet) \
    X(1, avcodec_version) \
    X(2, av_find_best_stream) \
    X(2, av_guess_sample_aspect_ratio) \
    X(2, av_interleaved_write_frame) \
    X(2, av_read_frame) \
    X(2, av_write_trailer) \
    X(2, avformat_alloc_output_context2) \
    X(2, avformat_close_input) \
    X(2, avformat_find_stream_info) \
    X(2, avformat_free_context) \
    X(2, avformat_new_stream) \
    X(2, avformat_open_input) \
    X(2, avformat_version) \
    X(2, avformat_write_header) \
    X(2, avio_closep) \
    X(2, avio_open) \
    X(3, av_buffersink_get_frame) \
    X(3, av_buffersink_get_time_base) \
    X(3, av_buffersrc_add_frame_flags) \
    X(3, av_buffersrc_parameters_alloc) \
    X(3, av_buffersrc_parameters_set) \
    X(3, avfilter_get_by_name) \
    X(3, avfilter_graph_alloc) \
    X(3, avfilter_graph_alloc_filter) \
    X(3, avfilter_graph_config) \
    X(3, avfilter_graph_create_filter) \
    X(3, avfilter_graph_free) \
    X(3, avfilter_init_str) \
    X(3, avfilter_link) \
    X(3, avfilter_version)

#define DECLARE_API(library, name) static decltype(&::name) name = nullptr;
FFMPEG_API(DECLARE_API)
#undef DECLARE_API

static const char* loadFFmpeg(const char* directory) {
    static std::string error;
    // Kept loaded for the process lifetime, like normal linked libraries. Every
    // lookup is restricted to its owning library, never RTLD_DEFAULT.
    static void* handles[4] = {};
    const char* names[] = {"avutil", "avcodec", "avformat", "avfilter"};
    const int majors[] = {LIBAVUTIL_VERSION_MAJOR, LIBAVCODEC_VERSION_MAJOR,
                          LIBAVFORMAT_VERSION_MAJOR, LIBAVFILTER_VERSION_MAJOR};
    for (int i = 0; i < 4; ++i) {
        std::string path = std::string(directory) + "/lib" + names[i] + "." + std::to_string(majors[i]) + ".dylib";
        handles[i] = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL | RTLD_FIRST);
        if (!handles[i]) {
            error = path + ": " + dlerror();
            return error.c_str();
        }
    }
#define LOAD_API(library, name) \
    name = reinterpret_cast<decltype(name)>(dlsym(handles[library], #name)); \
    if (!name) { error = std::string(#name) + ": " + dlerror(); return error.c_str(); }
    FFMPEG_API(LOAD_API)
#undef LOAD_API
    const unsigned versions[] = {avutil_version(), avcodec_version(), avformat_version(), avfilter_version()};
    const unsigned minimums[] = {AV_VERSION_INT(60, 8, 100), AV_VERSION_INT(62, 11, 100),
                                 AV_VERSION_INT(62, 3, 100), AV_VERSION_INT(11, 4, 100)};
    for (int i = 0; i < 4; ++i) {
        if ((versions[i] >> 16) != majors[i] || versions[i] < minimums[i]) {
            error = std::string(names[i]) + ": incompatible FFmpeg library version";
            return error.c_str();
        }
    }
    fprintf(stderr, "Metal export: loaded FFmpeg %s from %s\n", av_version_info(), directory);
    return nullptr;
}

#undef FFMPEG_API
} // namespace
