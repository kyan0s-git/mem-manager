#include "memsys.h"

#include <errno.h>
#include <libproc.h>
#include <mach/mach.h>
#include <mach/mach_time.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/sysctl.h>
#include <unistd.h>

int mm_vm_stats(mm_vmstats *out) {
    vm_statistics64_data_t s;
    mach_msg_type_number_t count = HOST_VM_INFO64_COUNT;
    kern_return_t kr = host_statistics64(mach_host_self(), HOST_VM_INFO64, (host_info64_t)&s, &count);
    if (kr != KERN_SUCCESS) {
        return (int)kr;
    }
    vm_size_t page = 0;
    host_page_size(mach_host_self(), &page);
    memset(out, 0, sizeof(*out));
    out->page_size = (uint64_t)(page ? page : vm_page_size);
    out->free_count = s.free_count;
    out->active_count = s.active_count;
    out->inactive_count = s.inactive_count;
    out->wire_count = s.wire_count;
    out->speculative_count = s.speculative_count;
    out->purgeable_count = s.purgeable_count;
    out->compressor_page_count = s.compressor_page_count;
    out->external_page_count = s.external_page_count;
    out->internal_page_count = s.internal_page_count;
    out->throttled_count = s.throttled_count;
    out->total_uncompressed_pages_in_compressor = s.total_uncompressed_pages_in_compressor;
    out->pageins = s.pageins;
    out->pageouts = s.pageouts;
    out->compressions = s.compressions;
    out->decompressions = s.decompressions;
    out->swapins = s.swapins;
    out->swapouts = s.swapouts;
    out->purges = s.purges;
    out->reactivations = s.reactivations;
    return 0;
}

uint64_t mm_memsize(void) {
    uint64_t v = 0;
    size_t len = sizeof(v);
    if (sysctlbyname("hw.memsize", &v, &len, NULL, 0) != 0) {
        return 0;
    }
    return v;
}

int mm_sysctl_int(const char *name, int *out) {
    int v = 0;
    size_t len = sizeof(v);
    if (sysctlbyname(name, &v, &len, NULL, 0) != 0) {
        return errno;
    }
    *out = v;
    return 0;
}

int mm_swap_usage(uint64_t *total, uint64_t *used) {
    struct xsw_usage u;
    size_t len = sizeof(u);
    if (sysctlbyname("vm.swapusage", &u, &len, NULL, 0) != 0) {
        return errno;
    }
    *total = u.xsu_total;
    *used = u.xsu_used;
    return 0;
}

int mm_list_pids(int *buf, int capacity) {
    int n = proc_listallpids(buf, capacity * (int)sizeof(int));
    return n;
}

mm_rusage mm_proc_rusage(int pid) {
    mm_rusage r;
    memset(&r, 0, sizeof(r));
    struct rusage_info_v4 ri;
    if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&ri) != 0) {
        r.err = errno;
        return r;
    }
    r.ok = 1;
    r.phys_footprint = ri.ri_phys_footprint;
    r.lifetime_max_phys_footprint = ri.ri_lifetime_max_phys_footprint;
    r.resident_size = ri.ri_resident_size;
    r.user_time_ns = ri.ri_user_time;
    r.system_time_ns = ri.ri_system_time;
    r.start_abstime = ri.ri_proc_start_abstime;
    return r;
}

int mm_proc_path(int pid, char *buf, int capacity) {
    int n = proc_pidpath(pid, buf, (uint32_t)capacity);
    return n > 0 ? n : 0;
}

int mm_proc_name(int pid, char *buf, int capacity) {
    int n = proc_name(pid, buf, (uint32_t)capacity);
    return n > 0 ? n : 0;
}

int mm_proc_bsdinfo(int pid, uint32_t *uid, int32_t *ppid) {
    struct proc_bsdshortinfo info;
    int n = proc_pidinfo(pid, PROC_PIDT_SHORTBSDINFO, 0, &info, (int)sizeof(info));
    if (n != (int)sizeof(info)) {
        return -1;
    }
    *uid = info.pbsi_uid;
    *ppid = (int32_t)info.pbsi_ppid;
    return 0;
}

int mm_pressure_trigger(int value) {
    if (sysctlbyname("kern.memorypressure_manual_trigger", NULL, NULL, &value, sizeof(value)) != 0) {
        return errno;
    }
    return 0;
}

double mm_abstime_to_ns(void) {
    mach_timebase_info_data_t tb;
    if (mach_timebase_info(&tb) != KERN_SUCCESS || tb.denom == 0) {
        return 1.0;
    }
    return (double)tb.numer / (double)tb.denom;
}
