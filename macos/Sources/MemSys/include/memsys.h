// MemSys: a tiny C shim over Darwin APIs (libproc, Mach VM statistics,
// sysctl) so the Swift code gets stable, simple types.
#ifndef MEMSYS_H
#define MEMSYS_H

#include <stdint.h>

typedef struct {
    uint64_t page_size;
    uint64_t free_count;
    uint64_t active_count;
    uint64_t inactive_count;
    uint64_t wire_count;
    uint64_t speculative_count;
    uint64_t purgeable_count;
    uint64_t compressor_page_count;
    uint64_t external_page_count;
    uint64_t internal_page_count;
    uint64_t throttled_count;
    uint64_t total_uncompressed_pages_in_compressor;
    uint64_t pageins;
    uint64_t pageouts;
    uint64_t compressions;
    uint64_t decompressions;
    uint64_t swapins;
    uint64_t swapouts;
    uint64_t purges;
    uint64_t reactivations;
} mm_vmstats;

typedef struct {
    int ok;
    int err;
    uint64_t phys_footprint;
    uint64_t lifetime_max_phys_footprint;
    uint64_t resident_size;
    uint64_t user_time_ns;
    uint64_t system_time_ns;
    uint64_t start_abstime;
} mm_rusage;

/// host_statistics64(HOST_VM_INFO64). Returns 0 on success.
int mm_vm_stats(mm_vmstats *out);

/// hw.memsize in bytes.
uint64_t mm_memsize(void);

/// Integer sysctl by name. Returns 0 on success.
int mm_sysctl_int(const char *name, int *out);

/// vm.swapusage. Returns 0 on success.
int mm_swap_usage(uint64_t *total, uint64_t *used);

/// proc_listallpids into buf; returns the number of pids (or -1).
int mm_list_pids(int *buf, int capacity);

/// proc_pid_rusage(RUSAGE_INFO_V4); same-UID or root only.
mm_rusage mm_proc_rusage(int pid);

/// proc_pidpath; returns the path length or 0.
int mm_proc_path(int pid, char *buf, int capacity);

/// proc_name; returns the name length or 0.
int mm_proc_name(int pid, char *buf, int capacity);

/// UID and parent PID via PROC_PIDT_SHORTBSDINFO. Returns 0 on success.
int mm_proc_bsdinfo(int pid, uint32_t *uid, int32_t *ppid);

/// Writes kern.memorypressure_manual_trigger (root only). Returns 0 or errno.
int mm_pressure_trigger(int value);

/// Mach absolute time → nanoseconds factor (numer/denom).
double mm_abstime_to_ns(void);

#endif
