import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { assertMacosDeployment, macosMinimumVersions } from "./macos-deployment.mjs";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const hook = path.join(repository, "scripts/bundle-macos-ffmpeg.mjs");
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...options });

test("macOS native FFmpeg dependencies relocate, survive repeated bundling and pass signature validation", { skip: process.platform !== "darwin" }, () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-ffmpeg-bundle-"));
  try {
    const source = path.join(directory, "probe.c");
    const binary = path.join(directory, "bili-shadowreplay");
    const runtime = path.join(directory, "runtime");
    fs.writeFileSync(source, String.raw`#include <stdio.h>
#include <stdlib.h>
#include <dlfcn.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libswresample/swresample.h>
#include <libavfilter/avfilter.h>
extern int vkEnumerateInstanceVersion(unsigned int *version);
int main(void) {
    unsigned int version = 0;
    if (vkEnumerateInstanceVersion(&version) != 0 || !version) return 1;
    if (getenv("BSR_TEST_BUNDLED_VULKAN")) {
        void *molten = dlopen("@executable_path/../Frameworks/libvulkan.dylib", RTLD_NOW | RTLD_LOCAL);
        if (!molten || dlopen("libvulkan.dylib", RTLD_NOW | RTLD_LOCAL) != molten ||
            !dlsym(molten, "vkExportMetalObjectsEXT")) {
            fprintf(stderr, "Bundled MoltenVK is not available through FFmpeg's loader name\n");
            return 1;
        }
    }
    const char *encoders[] = { "libopus", "libmp3lame", "libx264", "libx265", "aac", "flac", "pcm_s16le", "mjpeg", "png", "ass", "mov_text" };
    const char *muxers[] = { "opus", "mp3", "wav", "flac", "ipod", "mp4", "mpegts", "hls", "matroska" };
    const char *filters[] = { "subtitles", "overlay", "concat", "xfade", "acrossfade", "amix", "scale", "pad", "aresample", "color", "anullsrc" };
    for (size_t i = 0; i < sizeof(encoders) / sizeof(*encoders); i++) {
        if (!avcodec_find_encoder_by_name(encoders[i])) { fprintf(stderr, "FFmpeg SDK lacks encoder %s\n", encoders[i]); return 1; }
    }
    for (size_t i = 0; i < sizeof(muxers) / sizeof(*muxers); i++) {
        if (!av_guess_format(muxers[i], NULL, NULL)) { fprintf(stderr, "FFmpeg SDK lacks muxer %s\n", muxers[i]); return 1; }
    }
    for (size_t i = 0; i < sizeof(filters) / sizeof(*filters); i++) {
        if (!avfilter_get_by_name(filters[i])) { fprintf(stderr, "FFmpeg SDK lacks filter %s\n", filters[i]); return 1; }
    }
    printf("%u %u %u\n", avformat_version(), swresample_version(), avfilter_version());
    return 0;
}
`);
    const include = run("pkg-config", ["--variable=includedir", "libavformat"]).trim();
    const lib = run("pkg-config", ["--variable=libdir", "libavformat"]).trim();
    const molten = process.env.BSR_MOLTENVK_LIBRARY || path.join(run("brew", ["--prefix", "molten-vk"]).trim(), "lib/libMoltenVK.dylib");
    // Cover an SDK linked directly to MoltenVK as well as libplacebo's loader
    // dependency: both must resolve to a single bundled implementation.
    run("cc", [source, `-I${include}`, `-L${lib}`, "-lavformat", "-lavcodec", "-lswresample", "-lavfilter", molten, `-Wl,-rpath,${path.dirname(molten)}`, "-mmacosx-version-min=15.0", "-Wl,-headerpad_max_install_names", "-o", binary]);
    const expected = run(binary, []);
    const minimum = run("sw_vers", ["-productVersion"]).trim();
    const original = fs.readFileSync(binary);
    const rejected = spawnSync(process.execPath, [hook, "--binary", binary, "--runtime", runtime, "--minimum-system-version", "1.0"], { encoding: "utf8" });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /above bundle minimum 1\.0/);
    assert.deepEqual(fs.readFileSync(binary), original, "deployment rejection must precede binary mutation");
    assert.equal(fs.existsSync(runtime), false);
    const libraryCommands = run("otool", ["-l", path.join(lib, "libavformat.dylib")]);
    if (macosMinimumVersions(libraryCommands).some((version) => Number(version.split(".")[0]) > 15)) {
      // A compatible executable alone is insufficient: newer Homebrew dylibs
      // must be rejected before install-name rewriting or copying takes place.
      const incompatibleDependency = spawnSync(process.execPath, [hook, "--binary", binary, "--runtime", runtime], { encoding: "utf8" });
      assert.notEqual(incompatibleDependency.status, 0);
      assert.match(incompatibleDependency.stderr, /libavformat[^\n]*requires macOS [^\n]*above bundle minimum 15\.0/);
      assert.deepEqual(fs.readFileSync(binary), original);
      assert.equal(fs.existsSync(runtime), false);
    }
    // This temporary smoke bundle targets the actual local SDK/host, not the
    // product's macOS 15 release baseline. No Mach-O minimum is rewritten.
    const bundle = [hook, "--binary", binary, "--runtime", runtime, "--minimum-system-version", minimum];
    run(process.execPath, bundle);
    run(process.execPath, bundle);

    const application = path.join(directory, "Probe.app");
    const contents = path.join(application, "Contents");
    const packaged = path.join(contents, "MacOS/bili-shadowreplay");
    fs.mkdirSync(path.dirname(packaged), { recursive: true });
    fs.copyFileSync(binary, packaged);
    fs.cpSync(runtime, path.join(contents, "Frameworks"), { recursive: true });
    fs.writeFileSync(path.join(contents, "Info.plist"), '<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>bili-shadowreplay</string><key>CFBundleIdentifier</key><string>cn.vjoi.ffmpeg-smoke</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>1</string><key>LSMinimumSystemVersion</key><string>' + minimum + '</string></dict></plist>');
    run("codesign", ["--force", "--sign", "-", packaged]);
    run("codesign", ["--force", "--sign", "-", application]);
    run("codesign", ["--verify", "--deep", "--strict", application]);
    run(process.execPath, [hook, "--verify", application]);
    const env = { ...process.env, PATH: "/nonexistent", DYLD_PRINT_LIBRARIES: "1", BSR_TEST_BUNDLED_VULKAN: "1" };
    delete env.DYLD_LIBRARY_PATH;
    delete env.DYLD_FALLBACK_LIBRARY_PATH;
    const launched = spawnSync(packaged, [], { encoding: "utf8", env });
    assert.equal(launched.status, 0, launched.stderr);
    assert.equal(launched.stdout, expected);
    assert.match(launched.stderr, /Contents\/Frameworks\/libavformat/);
    assert.match(launched.stderr, /Contents\/Frameworks\/libswresample/);
    assert.match(launched.stderr, /Contents\/Frameworks\/libavfilter/);
    assert.match(launched.stderr, /Contents\/Frameworks\/libvulkan\.dylib/);
    assert.doesNotMatch(launched.stderr, /\/opt\/homebrew\/|\/usr\/local\/Cellar\//);

    // An explicit CI target must not reuse an old native executable.
    const oldNative = path.join(directory, "cargo-target/release/bili-shadowreplay");
    fs.mkdirSync(path.dirname(oldNative), { recursive: true });
    fs.copyFileSync(packaged, oldNative);
    const missingTarget = spawnSync(process.execPath, [hook], {
      encoding: "utf8",
      env: { ...process.env, CARGO_TARGET_DIR: path.join(directory, "cargo-target"), RUST_TARGET: "missing-apple-darwin" },
    });
    assert.notEqual(missingTarget.status, 0);
    assert.match(missingTarget.stderr, /Application executable not found/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});


test("Mach-O deployment checks cover modern, legacy and multi-architecture minima", () => {
  const modern = "Load command 1\ncmd LC_BUILD_VERSION\nplatform 1\nminos 27.0\nsdk 27.0\n";
  const legacy = "Load command 1\ncmd LC_VERSION_MIN_MACOSX\nversion 10.9\nsdk 15.0\n";
  assert.deepEqual(macosMinimumVersions(modern), ["27.0"]);
  assert.deepEqual(macosMinimumVersions(legacy), ["10.9"]);
  assert.doesNotThrow(() => assertMacosDeployment(legacy, "15.0", "old.dylib"));
  assert.doesNotThrow(() => assertMacosDeployment(modern, "27", "current.dylib"));
  assert.throws(() => assertMacosDeployment(modern, "15.0", "ffmpeg.dylib"), /ffmpeg\.dylib requires macOS 27\.0, above bundle minimum 15\.0/);
  assert.throws(() => assertMacosDeployment(legacy + modern, "15.0", "universal.dylib"), /above bundle minimum/);
  assert.throws(() => assertMacosDeployment("", "15.0", "unknown.dylib"), /Cannot determine/);
  assert.throws(() => assertMacosDeployment(modern, "invalid", "invalid.dylib"), /Invalid macOS version/);
  assert.throws(() => assertMacosDeployment(modern.replace("27.0", "15.10"), "15.9", "minor.dylib"), /above bundle minimum/);
});
