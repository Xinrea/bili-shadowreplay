// Lightweight, nonprivileged system context. No CPU/GPU frequency inference.
#import <Foundation/Foundation.h>
#import <IOKit/IOKitLib.h>
#include <mach/mach.h>
#include <unistd.h>
#include <cstdio>
#include <chrono>
int main() {
    setvbuf(stdout, nullptr, _IOLBF, 0);
    auto gpu = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("AGXAccelerator"));
    auto start = std::chrono::steady_clock::now();
    for (;;) { @autoreleasepool {
        host_cpu_load_info_data_t cpu{}; mach_msg_type_number_t size = HOST_CPU_LOAD_INFO_COUNT;
        auto host = mach_host_self(); auto result = host_statistics(host, HOST_CPU_LOAD_INFO, (host_info_t)&cpu, &size);
        mach_port_deallocate(mach_task_self(), host);
        double load[3]{}; getloadavg(load, 3);
        auto info = NSProcessInfo.processInfo;
        auto stats = gpu ? IORegistryEntryCreateCFProperty(gpu, CFSTR("PerformanceStatistics"), kCFAllocatorDefault, 0) : nullptr;
        id utilization = stats ? [(__bridge NSDictionary*)stats objectForKey:@"Device Utilization %"] : nil;
        NSString* gpuValue = [utilization isKindOfClass:NSNumber.class] ? [utilization stringValue] : @"null";
        auto t = std::chrono::duration<double>(std::chrono::steady_clock::now() - start).count();
        printf("{\"seconds\":%.3f,\"thermal_state\":%ld,\"low_power\":%s,\"load_1m\":%.3f,\"cpu_ticks\":[%u,%u,%u,%u],\"cpu_status\":%d,\"gpu_utilization_percent\":%s}\n",
            t, (long)info.thermalState, info.lowPowerModeEnabled ? "true" : "false", load[0],
            cpu.cpu_ticks[0], cpu.cpu_ticks[1], cpu.cpu_ticks[2], cpu.cpu_ticks[3], result, gpuValue.UTF8String);
        if (stats) CFRelease(stats);
    } sleep(1); }
}
