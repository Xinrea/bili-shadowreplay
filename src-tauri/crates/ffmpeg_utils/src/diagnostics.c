// Keep platform-specific va_list handling on the C side of the Rust boundary.
// These stable libavutil log declarations need no SDK structs or headers; the
// linked symbols come from the same ffmpeg-sys-next dependency as media work.
#include <stdarg.h>
#include <string.h>

extern void av_log_set_callback(void (*callback)(void *, int, const char *, va_list));
extern void av_log_default_callback(void *, int, const char *, va_list);
extern int av_log_format_line2(void *, int, const char *, va_list, char *, int, int *);
extern int av_log_get_level(void);
extern void bsr_ffmpeg_log_message(int level, const char *message);

static void log_callback(void *context, int level, const char *format, va_list args) {
    va_list stderr_args;
    va_copy(stderr_args, args);
    av_log_default_callback(context, level, format, stderr_args);
    va_end(stderr_args);

    // AV_LOG_WARNING = 24. Preserve existing stderr behavior for all levels,
    // but only warnings and errors belong in the application diagnostic log.
    level &= 0xff;
    if (level > 24 || level > av_log_get_level())
        return;

    char line[4096];
    int prefix = 1;
    int length = av_log_format_line2(context, level, format, args, line, sizeof(line), &prefix);
    if (length < 0)
        return;
    if (length >= (int)sizeof(line)) {
        const char marker[] = "... [truncated]";
        memcpy(line + sizeof(line) - sizeof(marker), marker, sizeof(marker));
    }
    bsr_ffmpeg_log_message(level, line);
}

// Called exactly once, before the first native media operation.
void bsr_install_ffmpeg_log_callback(void) {
    av_log_set_callback(log_callback);
}
