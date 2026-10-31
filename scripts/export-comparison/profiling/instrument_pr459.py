#!/usr/bin/env python3
"""Instrument a freshly extracted pinned PR459 tree for stage measurements.

Exact source hash verification rejects a different/already modified tree. By
default only timing is added. --without-cpu-activity explicitly removes the
busy-loop helper for a separate ablation; the manifest records this choice.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def instrument(root, without_cpu_activity=False):
    manifest = json.loads((root / 'source-manifest.json').read_text())
    assert manifest['verified_head'] == 'c73c7aaff9b9dd9abb561c90a48f19b2d6374112'
    for name, digest in manifest['files'].items():
        assert sha(root / 'source' / name) == digest, name
    native = root / 'source/src-tauri/crates/gpu-export/native'
    shutil.copyfile(Path(__file__).with_name('pr459_profile.hpp'), native / 'pr459_profile.hpp')
    def replace(source, old, new):
        assert source.count(old) == 1, (old, source.count(old))
        return source.replace(old, new)
    p = (native / 'pipeline.h').read_text()
    p = replace(p, '#include "ffmpeg_api.h"', '#include "ffmpeg_api.h"\n#include "pr459_profile.hpp"')
    p = replace(p, 'static void run(const ExportConfig& config) {', 'static void run(const ExportConfig& config) {\n    prprof::Run profile; prprof::Session session("render"); prprof::Scope root("worker");')
    for method, stage in [('bool push(Work value)', 'queue_push'), ('bool pop(Work& value)', 'queue_pop'), ('void push(AVFrame* frame)', 'cpu_filter'), ('bool pop(AVFrame* frame)', 'cpu_filter')]:
        p = replace(p, method + ' {', method + ' {\n        prprof::Scope scope("' + stage + '");')
    for thread in ('decode', 'encode'):
        p = replace(p, thread + 'Thread = std::thread([&] { BSR_POOL { try {', thread + 'Thread = std::thread([&] { prprof::Session session("' + thread + '"); prprof::Scope root("worker"); BSR_POOL { try {')
    for counter in ('decodedFrames', 'encodedFrames'):
        p = replace(p, '++' + counter + ';', '++' + counter + '; prprof::checkpoint(' + counter + ');')
    p = replace(p, 'pending.push_back(index); ++count;', 'pending.push_back(index); ++count; prprof::checkpoint(count);')
    # Wrap individual C API calls, leaving arguments, ordering and retry logic intact.
    stages = {'av_read_frame':'demux_read', 'avcodec_send_packet':'decode_send', 'avcodec_receive_frame':'decode_receive', 'avcodec_send_frame':'encode_send', 'avcodec_receive_packet':'encode_receive', 'av_interleaved_write_frame':'mux_write', 'av_write_trailer':'mux_finalize'}
    for api, stage in stages.items():
        positions = list(re.finditer(r'\b' + api + r'\(', p))
        for match in reversed(positions):
            depth, end = 1, match.end()
            while depth:
                depth += (p[end] == '(') - (p[end] == ')')
                end += 1
            call = p[match.start():end]
            p = p[:match.start()] + 'prprof::measure("' + stage + '", [&] { return ' + call + '; })' + p[end:]
    (native / 'pipeline.h').write_text(p)
    p = (native / 'metal.mm').read_text()
    for signature, stage in [('CVPixelBufferRef upload(FrameSlot& slot, const AVFrame* frame)', 'nv12_upload'), ('CVPixelBufferRef compose(FrameSlot& slot, CVPixelBufferRef source)', 'composite_submit'), ('void wait(FrameSlot& slot)', 'gpu_completion_wait'), ('Frame take(size_t index)', 'frame_handoff'), ('void submit(size_t index, AVFrame* cpu, double time)', 'process_frame')]:
        p = replace(p, signature + ' {', signature + ' {\n        prprof::Scope scope("' + stage + '");')
    p = replace(p, 'volatile uint64_t value = 1;', 'prprof::Session session("cpu_activity"); prprof::Scope root("worker");\n            volatile uint64_t value = 1;')
    draw = 'bsr_skia_metal_draw(renderer, time, (__bridge void*)slot.overlay)'
    p = replace(p, draw, 'prprof::measure("skia_render", [&] { return ' + draw + '; })')
    p = replace(p, 'Frame result(slot.frame);', 'prprof::observe("composite_gpu", (slot.command.GPUEndTime - slot.command.GPUStartTime) * 1000.0);\n        Frame result(slot.frame);')
    if without_cpu_activity:
        signature = '    explicit CPUActivity(unsigned logicalCPUs) {'
        start = p.index(signature)
        depth, end = 1, start + len(signature)
        while depth:
            depth += (p[end] == '{') - (p[end] == '}')
            end += 1
        p = p[:start] + signature + '\n        (void)logicalCPUs; // Isolated benchmark ablation.\n    }' + p[end:]
    (native / 'metal.mm').write_text(p)
    # Vulkan instrumentation adds host scopes only: no queue, fence, timeline,
    # quality or frame-lifetime changes, and no synthetic GPU timestamps.
    p = (native / 'vulkan.cpp').read_text()
    for signature, stage in [('void retire(Slot& slot)', 'frame_slot_wait'),
                             ('Frame allocate(AVBufferRef* poolRef)', 'frame_allocate'),
                             ('void submit(size_t index, AVFrame* cpu, double time)', 'process_frame')]:
        p = replace(p, signature + ' {', signature + ' {\n        prprof::Scope scope("' + stage + '");')
    calls = {
        'av_hwframe_transfer_data(slot.source.get(), cpu, 0)': 'nv12_upload',
        'bsr_skia_vulkan_draw(renderer, time, &image)': 'skia_render',
        'vk.WaitForFences(context->act_dev, 1, &slot.fence, VK_TRUE, UINT64_MAX)': 'gpu_completion_wait',
        'g.context->lock_queue(g.deviceContext, g.family, 0)': 'queue_lock',
    }
    for call, stage in calls.items():
        p = replace(p, call, 'prprof::measure("' + stage + '", [&] { return ' + call + '; })')
    p = replace(p, '        overlay->layout[0] = VK_IMAGE_LAYOUT_GENERAL;',
                '        prprof::Scope composite("composite_submit");\n        overlay->layout[0] = VK_IMAGE_LAYOUT_GENERAL;')
    p = replace(p, 'Frame take(size_t index) { return std::move(slots[index].output); }',
                'Frame take(size_t index) { prprof::Scope scope("frame_handoff"); return std::move(slots[index].output); }')
    (native / 'vulkan.cpp').write_text(p)
    result = {'base': manifest, 'without_cpu_activity': without_cpu_activity, 'instrumented_files': {str(f.relative_to(root)): sha(f) for f in (native / 'pipeline.h', native / 'metal.mm', native / 'vulkan.cpp', native / 'pr459_profile.hpp')}, 'instrumentation_sha256': sha(Path(__file__))}
    (root / 'instrumentation.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--without-cpu-activity', action='store_true', help='Benchmark ablation only: remove the busy-loop helper')
    args = parser.parse_args()
    instrument(args.root.resolve(strict=True), args.without_cpu_activity)
