//! NT native API: memory lists, performance counters, process snapshots,
//! pool tags, and the memory-management commands.
//!
//! Layouts follow phnt (`ntexapi.h`); see docs/research/windows-memory-internals.md.
//! Every call returns an NTSTATUS so callers can turn failures into capability flags.

use std::ffi::c_void;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret_len: *mut u32) -> i32;
    fn NtSetSystemInformation(class: u32, info: *mut c_void, len: u32) -> i32;
}

pub const SYSTEM_PERFORMANCE_INFORMATION: u32 = 2;
pub const SYSTEM_PROCESS_INFORMATION: u32 = 5;
pub const SYSTEM_POOLTAG_INFORMATION: u32 = 22;
pub const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
pub const SYSTEM_FILECACHE_INFORMATION_EX: u32 = 81;
pub const SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION: u32 = 130;

pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
pub const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023_u32 as i32;
pub const STATUS_PRIVILEGE_NOT_HELD: i32 = 0xC000_0061_u32 as i32;
pub const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003_u32 as i32;

pub fn ok(status: i32) -> bool {
    status >= 0
}

pub fn status_name(status: i32) -> &'static str {
    match status {
        STATUS_SUCCESS => "ok",
        STATUS_PRIVILEGE_NOT_HELD => "privilege not held",
        STATUS_INVALID_INFO_CLASS => "not supported",
        STATUS_INFO_LENGTH_MISMATCH => "length mismatch",
        s if s >= 0 => "ok",
        _ => "failed",
    }
}

/// `SYSTEM_MEMORY_LIST_INFORMATION` (176 bytes on 64-bit).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryLists {
    pub zero: usize,
    pub free: usize,
    pub modified: usize,
    pub modified_no_write: usize,
    pub bad: usize,
    pub standby_by_priority: [usize; 8],
    pub repurposed_by_priority: [usize; 8],
    pub modified_pagefile: usize,
}

impl MemoryLists {
    pub fn standby(&self) -> usize {
        self.standby_by_priority.iter().sum()
    }
}

pub fn memory_lists() -> Result<MemoryLists, i32> {
    let mut v = MemoryLists::default();
    let mut ret = 0u32;
    let st = unsafe {
        NtQuerySystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            (&mut v as *mut MemoryLists).cast(),
            size_of::<MemoryLists>() as u32,
            &mut ret,
        )
    };
    if ok(st) { Ok(v) } else { Err(st) }
}

/// Leading part of `SYSTEM_PERFORMANCE_INFORMATION`; the full structure is
/// queried into a larger buffer and only these fields are read.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PerfInfo {
    pub idle_process_time: i64,
    pub io_read_transfer: i64,
    pub io_write_transfer: i64,
    pub io_other_transfer: i64,
    pub io_read_ops: u32,
    pub io_write_ops: u32,
    pub io_other_ops: u32,
    pub available_pages: u32,
    pub committed_pages: u32,
    pub commit_limit: u32,
    pub peak_commitment: u32,
    pub page_fault_count: u32,
    pub copy_on_write_count: u32,
    pub transition_count: u32,
    pub cache_transition_count: u32,
    pub demand_zero_count: u32,
    pub page_read_count: u32,
    pub page_read_io_count: u32,
    pub cache_read_count: u32,
    pub cache_io_count: u32,
    pub dirty_pages_write_count: u32,
    pub dirty_write_io_count: u32,
    pub mapped_pages_write_count: u32,
    pub mapped_write_io_count: u32,
    pub paged_pool_pages: u32,
    pub non_paged_pool_pages: u32,
    pub paged_pool_allocs: u32,
    pub paged_pool_frees: u32,
    pub non_paged_pool_allocs: u32,
    pub non_paged_pool_frees: u32,
}

/// Generous buffer: the real structure is ~0x160 bytes and grows across releases.
const PERF_BUF: usize = 1024;

pub fn perf_info() -> Result<PerfInfo, i32> {
    let mut buf = [0u64; PERF_BUF / 8];
    let mut ret = 0u32;
    let mut len = 0x158u32; // size of the Windows 10/11 structure
    for _ in 0..3 {
        let st = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PERFORMANCE_INFORMATION,
                buf.as_mut_ptr().cast(),
                len,
                &mut ret,
            )
        };
        if ok(st) {
            // SAFETY: PerfInfo is a prefix of the queried structure and the
            // buffer is 8-byte aligned and larger than PerfInfo.
            return Ok(unsafe { std::ptr::read(buf.as_ptr().cast::<PerfInfo>()) });
        }
        if st == STATUS_INFO_LENGTH_MISMATCH
            && ret as usize > 0
            && (ret as usize) <= PERF_BUF
            && ret != len
        {
            len = ret;
            continue;
        }
        if st == STATUS_INFO_LENGTH_MISMATCH && len as usize != PERF_BUF {
            len = PERF_BUF as u32;
            continue;
        }
        return Err(st);
    }
    Err(STATUS_INFO_LENGTH_MISMATCH)
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

/// Header of `SYSTEM_PROCESS_INFORMATION` up to `PrivatePageCount`.
#[repr(C)]
struct RawProcess {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: u64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UnicodeString,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
}

/// One process from a snapshot. `name` borrows the snapshot buffer.
pub struct ProcEntry<'a> {
    pub pid: u32,
    pub parent_pid: u32,
    pub create_time: i64,
    /// User + kernel time, 100 ns units.
    pub cpu_time: u64,
    pub working_set: u64,
    /// Private bytes (commit charged to the process).
    pub private_bytes: u64,
    pub session_id: u32,
    pub name: &'a [u16],
}

/// Fills `buf` (grow-only, reused) with a `SystemProcessInformation` snapshot.
pub fn query_processes(buf: &mut Vec<u64>) -> Result<(), i32> {
    if buf.is_empty() {
        buf.resize(64 * 1024, 0); // 512 KiB
    }
    for _ in 0..6 {
        let mut ret = 0u32;
        let bytes = (buf.len() * 8) as u32;
        let st = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESS_INFORMATION,
                buf.as_mut_ptr().cast(),
                bytes,
                &mut ret,
            )
        };
        if ok(st) {
            return Ok(());
        }
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_TOO_SMALL {
            let want = (ret as usize).max(bytes as usize) * 3 / 2;
            buf.resize(want.div_ceil(8), 0);
            continue;
        }
        return Err(st);
    }
    Err(STATUS_INFO_LENGTH_MISMATCH)
}

/// Walks a snapshot produced by [`query_processes`].
pub fn for_each_process(buf: &[u64], mut f: impl FnMut(&ProcEntry)) {
    let base = buf.as_ptr().cast::<u8>();
    let limit = buf.len() * 8;
    let mut off = 0usize;
    loop {
        if off + size_of::<RawProcess>() > limit {
            break;
        }
        // SAFETY: the kernel wrote a chain of SYSTEM_PROCESS_INFORMATION records
        // into `buf`; each record is 8-byte aligned and `off` stays in bounds.
        let p = unsafe { &*base.add(off).cast::<RawProcess>() };
        let name: &[u16] = if p.image_name.buffer.is_null() || p.image_name.length == 0 {
            &[]
        } else {
            // SAFETY: ImageName.Buffer points into the same snapshot buffer.
            unsafe {
                std::slice::from_raw_parts(p.image_name.buffer, p.image_name.length as usize / 2)
            }
        };
        f(&ProcEntry {
            pid: p.unique_process_id as u32,
            parent_pid: p.inherited_from_unique_process_id as u32,
            create_time: p.create_time,
            cpu_time: (p.user_time as u64).wrapping_add(p.kernel_time as u64),
            working_set: p.working_set_size as u64,
            private_bytes: p.private_page_count as u64,
            session_id: p.session_id,
            name,
        });
        if p.next_entry_offset == 0 {
            break;
        }
        off += p.next_entry_offset as usize;
    }
}

/// `SYSTEM_MEMORY_LIST_COMMAND`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemCommand {
    EmptyWorkingSets = 2,
    FlushModifiedList = 3,
    PurgeStandbyList = 4,
    PurgeLowPriorityStandbyList = 5,
}

pub fn mem_command(cmd: MemCommand) -> i32 {
    let mut c = cmd as u32;
    unsafe {
        NtSetSystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            (&mut c as *mut u32).cast(),
            4,
        )
    }
}

#[repr(C)]
#[derive(Default)]
struct FileCacheInfo {
    current_size: usize,
    peak_size: usize,
    page_fault_count: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    current_size_including_transition: usize,
    peak_size_including_transition: usize,
    transition_repurpose_count: u32,
    flags: u32,
}

/// Trims the system file-cache working set (needs SeIncreaseQuotaPrivilege).
pub fn flush_file_cache() -> i32 {
    let mut info = FileCacheInfo {
        minimum_working_set: usize::MAX,
        maximum_working_set: usize::MAX,
        ..Default::default()
    };
    unsafe {
        NtSetSystemInformation(
            SYSTEM_FILECACHE_INFORMATION_EX,
            (&mut info as *mut FileCacheInfo).cast(),
            size_of::<FileCacheInfo>() as u32,
        )
    }
}

#[repr(C)]
struct CombineInfoEx {
    handle: usize,
    pages_combined: usize,
    flags: u32,
}

/// Page combining (Windows 10+). Returns the status and pages combined.
pub fn combine_pages() -> (i32, usize) {
    let mut info = CombineInfoEx {
        handle: 0,
        pages_combined: 0,
        flags: 0,
    };
    let st = unsafe {
        NtSetSystemInformation(
            SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION,
            (&mut info as *mut CombineInfoEx).cast(),
            size_of::<CombineInfoEx>() as u32,
        )
    };
    (st, info.pages_combined)
}

/// One `SYSTEM_POOLTAG` entry.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PoolTag {
    pub tag: [u8; 4],
    pub paged_allocs: u32,
    pub paged_frees: u32,
    pub paged_used: usize,
    pub nonpaged_allocs: u32,
    pub nonpaged_frees: u32,
    pub nonpaged_used: usize,
}

impl PoolTag {
    pub fn used(&self) -> u64 {
        (self.paged_used + self.nonpaged_used) as u64
    }
    pub fn tag_str(&self) -> String {
        self.tag
            .iter()
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '?'
                }
            })
            .collect()
    }
}

pub fn pool_tags() -> Result<Vec<PoolTag>, i32> {
    let mut buf: Vec<u64> = vec![0; 16 * 1024];
    for _ in 0..6 {
        let mut ret = 0u32;
        let bytes = (buf.len() * 8) as u32;
        let st = unsafe {
            NtQuerySystemInformation(
                SYSTEM_POOLTAG_INFORMATION,
                buf.as_mut_ptr().cast(),
                bytes,
                &mut ret,
            )
        };
        if ok(st) {
            let count = (buf[0] & 0xFFFF_FFFF) as usize;
            let max = (buf.len() * 8 - 8) / size_of::<PoolTag>();
            let count = count.min(max);
            // SAFETY: entries start at offset 8 (ULONG Count + padding) and the
            // count is clamped to the buffer.
            let tags =
                unsafe { std::slice::from_raw_parts(buf.as_ptr().add(1).cast::<PoolTag>(), count) };
            return Ok(tags.to_vec());
        }
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_TOO_SMALL {
            let want = (ret as usize).max(bytes as usize * 2);
            buf.resize(want.div_ceil(8), 0);
            continue;
        }
        return Err(st);
    }
    Err(STATUS_INFO_LENGTH_MISMATCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(size_of::<MemoryLists>(), 22 * size_of::<usize>());
        assert_eq!(size_of::<PoolTag>(), 40);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::offset_of!(RawProcess, private_page_count), 0xC8);
    }
}

#[repr(C)]
struct OsVersionInfo {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
}

/// Windows build number (e.g. 22631), via RtlGetVersion (not subject to manifest lies).
pub fn os_build() -> u32 {
    let mut v = OsVersionInfo {
        size: size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        csd: [0; 128],
    };
    if ok(unsafe { RtlGetVersion(&mut v) }) {
        v.build
    } else {
        0
    }
}
