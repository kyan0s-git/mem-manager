# Detecting memory leaks and runaway processes from the outside

> A memory manager cannot *fix* a leak. Only the leaking program, or a restart, can. What it
> **can** do is notice a leak early and reliably, name the culprit, estimate the time until
> trouble, and offer the least-disruptive remedy. This document covers what to measure, the
> statistics, and how to avoid crying wolf.
> Tags refer to [`sources.md`](sources.md).

## 1. Taxonomy

| Kind | Where it shows up (Windows) | Where it shows up (macOS) | Remedy |
|---|---|---|---|
| **User-mode heap leak** | Process **Private Bytes** (`PrivatePageCount`) / commit rises | Process **`phys_footprint`** rises | Restart the process |
| **Handle / GDI / USER object leak** | `HandleCount` rises, often with pool growth | (fd / Mach port leaks rarely matter for RAM) | Restart the process |
| **Cache that never evicts** (app bug or policy) | Private bytes rise and then plateau at a high value | Footprint plateaus high | Usually not a leak; optionally restart |
| **Kernel pool leak (driver)** | Paged or nonpaged pool grows; all processes flat [W14] | Wired grows with no app attribution | Identify the driver; reboot or update the driver |
| **File-cache / metafile bloat** | System cache working set grows | — (file cache is fully reclaimable) | Cache cap (Windows aggressive mode) |
| **Runaway (not a leak)** | Fast growth from a legitimate workload (compile, VM, ML) | Same | Inform only, unless pressure is critical |

**Why not working set / RSS?** The working set goes down when trimmed or compressed, and goes up
on demand paging of shared code. It measures *residency*, not *ownership*. Private bytes on
Windows and phys_footprint on macOS measure memory the process *owns* and that cannot be
reclaimed without its cooperation [M12][G3].

## 2. Sampling budget

We have to watch hundreds of processes for hours without using memory ourselves:

1. **Cheap system-wide snapshot every T₁ seconds.** One `SystemProcessInformation` call on
   Windows; `proc_listallpids` plus `proc_pid_rusage` per same-UID PID on macOS. Both run in
   about a millisecond.
2. **Candidate set.** Keep detailed history only for the **top K processes** (default K = 24) by
   current private or footprint size, **plus** any process whose short-term growth exceeds a fast
   threshold. Everything else keeps only a 2-value summary: first-seen size and EWMA.
3. **Per-candidate ring buffer.** One sample per 30 s over 2 h is 240 samples. Each is a `u32`
   value in 64 KB units plus a `u16` time delta, so 6 bytes × 240 = **1.4 KB per process** and
   about 35 KB for K = 24. Older history is downsampled into a 24 h ring of 5-minute medians
   (288 × 6 B).
4. **Identity.** Key by (PID, process start time) so PID reuse doesn't merge histories. App-level
   aggregation (e.g. all `Google Chrome Helper (Renderer)` processes) happens in a second pass for
   presentation.

## 3. Statistics

Memory series are noisy. GC sawtooth, cache fills and sudden drops all break ordinary
least-squares regression, which is sensitive to outliers. We use two robust, non-parametric tools.

### 3.1 Theil–Sen slope [G1]

```
slope = median over i<j of (y_j − y_i) / (t_j − t_i)
```

- Robust to up to about 29% outliers (GC spikes, one-off allocations).
- O(n²) pairs. For n = 240 that is 28,680 slopes, still under 0.2 ms. To bound cost and avoid
  allocation, we run it on a **decimated series** (n ≤ 64 → 2,016 pairs). The pairwise slopes go
  into a fixed 8 KB scratch array, and the median is found with `select_nth_unstable` (or
  quickselect in Swift), which is O(n²) overall with no heap allocation.
- The intercept is the median of `y_i − slope × t_i`.

### 3.2 Mann–Kendall trend test [G2]

```
S = Σ_{i<j} sign(y_j − y_i)
τ = S / (n(n−1)/2)                       (Kendall's tau, −1..1)
Var(S) = n(n−1)(2n+5)/18                 (ignoring ties; tie correction applied)
Z = (S − sign(S)) / sqrt(Var(S))
```

- τ close to 1 means the series almost always goes up, which is the signature of a leak.
- Z gives significance; we require p < 0.01 (Z > 2.33, one-sided).
- Because it is computed over the same pairs, Theil–Sen and Mann–Kendall share one loop.

### 3.3 Decision rule (defaults; profile-dependent)

A process is **flagged as leaking** when **all** of the following hold over the analysis window
(≥ 30 min of data, ≥ 40 samples):

| Condition | Balanced default | Rationale |
|---|---|---|
| Theil–Sen slope | ≥ 100 MB/h, or ≥ 5%/h of total RAM ÷ 16 | Ignores slow, benign drift |
| Total growth from window start | ≥ max(256 MB, 25% of the baseline size) | Ignores small apps growing from tiny bases |
| Kendall τ | ≥ 0.6 | Mostly monotonic, not a sawtooth |
| Mann–Kendall Z | > 2.33 | Statistically significant |
| No plateau | Slope of the last 25% of the window ≥ 50% of the overall slope | Excludes caches that have filled up and stopped |

It is classified as a **runaway** (fast but possibly legitimate) when the slope is at least
1 GB/min over 2 min. That gets an *informational* badge, and becomes a notification only if
pressure is High or Critical.

**Time-to-trouble.** For flagged processes, estimate when the system will reach the commit limit
(Windows) or Critical pressure (macOS), assuming the slope continues:
`ETA = headroom / slope`. The notification leads with this: "Slack is leaking about 380 MB/h. At
this rate you'll run low on memory in about 3 h."

### 3.4 False-positive controls

- **Warm-up exclusion.** Ignore the first 10 minutes after process start. Startup allocation is
  not a leak.
- **Workload correlation** [G4]. If the process's CPU time over the window is near zero but memory
  grows, that is *more* suspicious. If growth stops when CPU stops, it is less suspicious. In v1
  this is a weak tiebreaker; we record it for later tuning.
- **Known-heavy apps.** Browsers, IDEs, VMs, games and ML runtimes get a 2× growth threshold by
  default. A user "Ignore" adds the app to a persistent ignore list (keyed by executable path or
  bundle ID, not by PID).
- **Hysteresis on alerts.** One notification per process per 6 h. Re-alert only if the ETA halves.
- **Test vectors.** `spec/test-vectors/leak-*.json` contains synthetic series: linear leak,
  sawtooth GC, cache fill and plateau, step allocation, and noisy flat. Both implementations must
  produce identical decisions (see `docs/architecture/policy-engine.md`).

## 4. Kernel pool leaks (Windows)

- Sample `SystemPerformanceInformation` (paged and nonpaged pool pages) every minute. Apply the
  same Theil–Sen/MK rule with thresholds scaled down (≥ 50 MB/h, ≥ 200 MB total).
- When flagged, take **two `SystemPoolTagInformation` snapshots** 5 minutes apart. Rank tags by
  `Δ(Used)` and by `Allocs − Frees` growth. Report the top 3 tags.
- Map tags to drivers with an embedded table of common tags (a few KB). If that fails, fall back to
  an on-demand scan of `System32\drivers\*.sys` for the 4-byte tag. That is what `strings.exe`
  triage does [W14].
- The message must be honest: "This memory is held by a driver and can't be freed without a
  reboot. Updating the driver usually fixes it."

## 5. macOS-specific notes

- Only same-UID processes are visible [M4]. That covers the important cases.
- **WebContent and Electron helpers.** Group them by their responsible app, using the
  responsible-PID API (`responsibility_get_pid_responsible_for_pid`, private) or a parent-chain
  plus bundle-path heuristic. The remedy is then phrased per app ("Reload heavy Safari tabs" /
  "Relaunch Slack").
- `ri_lifetime_max_phys_footprint` lets us show the peak without storing more history.
- When the pressure source fires *Critical*, we immediately run the leak detector on the current
  ring buffers so the notification can name the cause right away.

## 6. Remedies, ordered by disruption

1. **Tell the user.** Name, slope, ETA, and an "Ignore this app" option.
2. **Graceful restart.**
   - macOS: `terminate()` then relaunch.
   - Windows: `WM_CLOSE` to top-level windows, wait, then relaunch from the image path if the app
     supports it.
3. **Containment (Windows only, user-chosen).**
   - Hard working-set cap [W3], plus memory priority VERY_LOW [W2]. This **delays** pressure on
     everyone else but does **not** stop commit growth. The UI says so.
   - A Job object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY` *would* cap commit, but assigning an
     already-running foreign process to a job can break apps that already use jobs (browsers). It
     is deliberately **not** offered in v1.
4. **Force quit.** Only on explicit user action.
