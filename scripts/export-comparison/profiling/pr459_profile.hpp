#pragma once
// Benchmark-only instrumentation; never installed in the production exporter.
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <vector>
#include <fstream>
#include <map>
#if defined(__APPLE__)
#include <pthread.h>
#elif defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif
#include <time.h>
#include <cstdlib>
#include <sstream>
namespace prprof {
using Clock = std::chrono::steady_clock;
inline double wall() { return std::chrono::duration<double, std::milli>(Clock::now().time_since_epoch()).count(); }
inline double cpu() {
#if defined(__APPLE__)
    return clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID) / 1e6;
#elif defined(_WIN32)
    FILETIME created{}, exited{}, kernel{}, user{};
    if (!GetThreadTimes(GetCurrentThread(), &created, &exited, &kernel, &user))
        throw std::runtime_error("GetThreadTimes failed");
    const auto ticks = [](FILETIME v) { return (uint64_t(v.dwHighDateTime) << 32) | v.dwLowDateTime; };
    return (ticks(kernel) + ticks(user)) / 10000.0;
#else
    timespec value{};
    if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &value)) throw std::runtime_error("thread CPU clock failed");
    return value.tv_sec * 1000.0 + value.tv_nsec / 1e6;
#endif
}
inline const char* platform() {
#if defined(__APPLE__)
    return "macos";
#elif defined(_WIN32)
    return "windows";
#else
    return "linux";
#endif
}
struct Timing { uint64_t calls = 0; double inclusive = 0, exclusive = 0, cpu = 0, exclusive_cpu = 0, maximum = 0; };
struct Child { double wall = 0, cpu = 0; };
struct Report {
    std::string name; unsigned qos = 0; int relative = 0;
    double start = wall(), cpu_start = cpu();
    std::map<std::string, Timing> spans, observations;
    std::vector<Child> children;
    std::vector<std::string> checkpoints;
};
inline thread_local Report* active = nullptr;
inline std::mutex reports_mutex;
inline std::vector<Report> reports;
inline bool enabled = false;
struct Scope {
    Report* report; const char* name; double start = 0, cpu_start = 0;
    explicit Scope(const char* n) : report(active), name(n) {
        if (report) { report->children.push_back({}); start = wall(); cpu_start = cpu(); }
    }
    ~Scope() {
        if (!report) return;
        double elapsed = wall() - start, used = cpu() - cpu_start;
        auto child = report->children.back(); report->children.pop_back();
        auto& t = report->spans[name]; ++t.calls; t.inclusive += elapsed; t.exclusive += elapsed - child.wall;
        t.cpu += used; t.exclusive_cpu += used - child.cpu; t.maximum = std::max(t.maximum, elapsed);
        if (!report->children.empty()) { auto& p = report->children.back(); p.wall += elapsed; p.cpu += used; }
    }
};
template<class F> auto measure(const char* name, F&& f) { Scope scope(name); return f(); }
struct Session {
    std::unique_ptr<Report> report;
    explicit Session(const char* name) {
        if (!enabled) return;
        report = std::make_unique<Report>(); report->name = name;
#if defined(__APPLE__)
        qos_class_t qos{};
        pthread_get_qos_class_np(pthread_self(), &qos, &report->relative);
        report->qos = static_cast<unsigned>(qos);
#endif
        active = report.get();
    }
    ~Session() {
        if (!report) return;
        active = nullptr; std::lock_guard lock(reports_mutex); reports.push_back(std::move(*report));
    }
};
inline void checkpoint(uint64_t frames) {
    if (!active || frames % 600 || active->checkpoints.size() >= 512) return;
    std::ostringstream out; out.precision(12);
    out << "{\"frames\":" << frames << ",\"wall_ms\":" << wall() - active->start
        << ",\"thread_cpu_ms\":" << cpu() - active->cpu_start << ",\"exclusive_ms\":{";
    bool first = true;
    for (const auto& [name, t] : active->spans) { if (!first) out << ','; first = false; out << '"' << name << "\":" << t.exclusive; }
    out << "}}"; active->checkpoints.push_back(out.str());
}
inline void observe(const char* name, double milliseconds) {
    if (!active) return;
    auto& t = active->observations[name]; ++t.calls; t.inclusive += milliseconds;
}
struct Run {
    const char* path;
    Run() : path(std::getenv("BSR_STAGE_PROFILE")) { enabled = path && *path; reports.clear(); }
    ~Run() {
        if (!enabled) return;
        std::ofstream out(path); out.precision(12); out << "{\"platform\":\"" << platform() << "\",\"threads\":["; bool first = true;
        for (const auto& r : reports) {
            if (!first) out << ','; first = false;
            out << "{\"name\":\"" << r.name << "\",\"requested_qos\":" << r.qos << ",\"relative_priority\":" << r.relative << ",\"spans\":{";
            bool item = true;
            for (const auto& [name, t] : r.spans) {
                if (!item) out << ','; item = false;
                out << '"' << name << "\":{\"calls\":" << t.calls << ",\"inclusive_ms\":" << t.inclusive
                    << ",\"exclusive_ms\":" << t.exclusive << ",\"inclusive_cpu_ms\":" << t.cpu
                    << ",\"exclusive_cpu_ms\":" << t.exclusive_cpu << ",\"max_ms\":" << t.maximum << '}';
            }
            out << "},\"observations\":{"; item = true;
            for (const auto& [name, t] : r.observations) { if (!item) out << ','; item = false; out << '"' << name << "\":{\"calls\":" << t.calls << ",\"inclusive_ms\":" << t.inclusive << '}'; }
            out << "},\"checkpoints\":["; item = true;
            for (const auto& p : r.checkpoints) { if (!item) out << ','; item = false; out << p; }
            out << "]}";
        }
        out << "]}\n"; enabled = false;
    }
};
}
