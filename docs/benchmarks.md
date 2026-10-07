# Benchmarks: how much memory MemManager frees, and what it costs

Each scenario creates real bloat on a GitHub-hosted runner, runs one MemManager action through
the same code path the app uses, and measures:

- how much memory became free;
- what that costs afterwards: page faults, re-read time, and re-touch time.

The **cost column matters as much as the freed column**. Freeing cache that will be needed again
only moves the cost to later. That is why MemManager runs these actions only under the
conditions in [policy-engine.md](architecture/policy-engine.md), and backs off when the cost
shows up.

Reproduce with `memmanager.exe --bench` from an elevated prompt, or `sudo MemManager --bench`.
The tables are rendered by [`scripts/bench_report.py`](../scripts/bench_report.py), and every
release's notes include a fresh run.

## Windows

The runner had 16 GiB of RAM and ran build 26100, elevated. The bloat was three idle processes
holding 2 GiB of private memory, plus a 2 GiB file read through the cache.

| Bloat | Action | Freed | % of RAM | Cost afterwards |
|---|---|---:|---:|---|
| Idle apps holding private memory | Selective trim of idle apps (tier 4) | 2.00 GiB | 12.5% | re-touch 514 ms vs 7 ms warm; 0 hard page faults |
| Modified (dirty) pages from trimmed apps | Flush modified list (tier 4) | 1.63 GiB | 10.2% | pagefile writes; pages become reusable cache |
| File cache after reading a large file | Clear low-priority cache (tier 2) | 0.00 GiB | 0.0% | none: only cache Windows was about to discard |
| File cache after reading a large file | Clear standby cache (tier 3) | 5.84 GiB | 36.5% | re-reading the file took 4905 ms vs 258 ms from cache |
| All of the above at once | Deep clean (manual) | 5.28 GiB | 33.0% | 623,923 hard faults in the next 10 s (baseline 11,971); apps re-touch 18.4 s |
| Identical pages across the system | Combine pages (tier 5) | 0.01 GiB | 0.1% | CPU scan only |

**What the numbers mean:**

- **Trimming idle apps** moved 2 GiB out of their working sets. When the apps touched that memory
  again, it came back from the standby and modified lists as soft faults, with no disk reads. The
  cost was 0.5 s spread over 2 GiB. This is the cheap, reversible kind of reclaim the Windows
  policy relies on under physical pressure.
- **Flushing the modified list** writes dirty pages to the pagefile, so 1.6 GiB can be reused
  without a later write stall.
- **Clearing the standby cache** freed the most memory, 5.8 GiB, and it is the most expensive
  action: the file then took **19× longer** to read. It only runs at High pressure, when Windows
  is already repurposing its own cache.
- **Tier 2** (priority-0 standby) freed nothing here. That is the expected result: it only clears
  cache that Windows itself had marked as least valuable, and an idle runner had less than 1 MB
  of it. On a long-running desktop this list grows to hundreds of MB.
- **Deep clean** shows why "RAM cleaners" that empty everything are a bad idea as a routine. It
  freed a third of RAM, then caused **52× the normal hard-fault rate** for the next 10 seconds as
  everything paged back in. MemManager offers it only as a manual action, with a confirmation.

## macOS

The runner had 7 GiB of RAM; the bench ran as root, the same privileges as the optional helper.
Each bloat source was about 0.7 GiB (10% of RAM).

| Bloat | Action | Freed | % of RAM | Cost afterwards |
|---|---|---:|---:|---|
| Volatile purgeable memory (app caches) | Nudge (helper) | 0.70 GiB | 10.0% | apps regenerate purged caches on next use |
| App holding a cache, cooperative | Nudge (helper) | 0.70 GiB | 10.0% | the app rebuilds its cache when needed |
| File cache after reading a large file | Purge disk cache (manual, helper) | 2.83 GiB | 40.4% | re-reading the file took 923 ms vs 121 ms from cache |
| Idle app holding memory | Quit idle app (suggestion / allowlist) | 0.70 GiB | 10.0% | the app must be reopened |

**What the numbers mean:**

- **Nudge** raises the kernel pressure level to *warning* for 3 seconds:
  - The kernel emptied the volatile purgeable region; its state went from VOLATILE to EMPTY.
  - The cooperative app released its whole cache, going from 753 MB to 1.8 MB.
  - The pressure level read back as normal (1) afterwards, confirming the latch reset.

  Nudge reclaims only what apps have volunteered to give up, so it complements XNU rather than
  fighting it. Apps that ignore pressure notifications give back nothing.
- **Purge** frees file cache, as `purge` does. The re-read became **7.6× slower**, so it stays a
  manual tool, useful for benchmarking but not for speed.
- **Quitting an idle heavy app** returns its whole footprint. MemManager suggests it and never
  does it automatically, except for apps on your allowlist.

## So how much can it reduce memory bloat?

- **Cache bloat** (standby, file cache, purgeable memory) can be freed almost entirely:
  - On these runners: 36% of RAM on Windows and 40% on macOS.
  - That memory was cache, though. The OS would have handed it to an app on demand anyway, and
    freeing it early makes the next read slower.
  - MemManager frees it only when the system is short of fast memory.
- **Idle-app bloat** is where the real wins are:
  - Windows: 2 GiB was moved out of idle apps, and paged back in for 0.5 s of soft faults.
  - macOS: cooperative apps gave back 100% of their cache when nudged.
- **Leaks** cannot be freed from outside the leaking process on either OS: the memory is still
  referenced. MemManager detects them (Theil–Sen slope with a Mann–Kendall trend test) and
  offers to restart the app, which returns the whole leaked amount.
