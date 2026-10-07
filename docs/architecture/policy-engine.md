# Policy engine: the shared specification

> Both apps implement this spec: Windows in Rust (`windows/core`), macOS in Swift
> (`macos/Sources/MemCore`). It is deliberately **platform-neutral**. Platform adapters turn OS
> data into `Sample`s and execute `Action`s; the engine decides.
> `spec/reference/policy_ref.py` is the executable reference. `spec/test-vectors/*.json` are
> generated from it, and both native implementations must reproduce every vector **exactly**
> (decisions) or within `1e-9` relative (numbers).

## 1. Units and conventions

- Time: `t` in milliseconds from an arbitrary monotonic origin (`u64`). Durations in the spec are
  given in seconds.
- Memory: **bytes** (`u64`) in samples; **MiB** (`f64`) inside the leak statistics.
- Rates: per second, unless the name says `_per_h`.
- Ratios are `f64` in [0, 1]. `clamp01(x) = min(1, max(0, x))`.

## 2. Inputs

```text
Sample {
  t_ms,
  total_bytes,                 // physical RAM
  fast_avail_bytes,            // Win: free + zero + standby[prio 0]      mac: free (incl. speculative)
  cache_bytes,                 // Win: standby (all prios)                 mac: external + purgeable
  commit_bytes, commit_limit,  // Win only (mac: 0, 0)
  churn_pages_cum,             // Win: Σ RepurposedPagesByPriority[5..7]   mac: decompressions (cumulative)
  hard_faults_cum,             // Win: PageReadCount                       mac: pageins
  kernel_level,                // mac: 1|2|4 from kern.memorystatus_vm_pressure_level; Win: 0
  free_pct,                    // mac: kern.memorystatus_level; Win: 0
  swap_used_bytes,             // mac: vm.swapusage.xsu_used; Win: 0
}
ProcSample { key: (pid, start_time), name, owned_bytes }   // Win: PrivatePageCount, mac: phys_footprint
```

## 3. Pressure score P ∈ [0, 1]

Each platform computes sub-scores, and `P` is the **maximum** of them. A single bad dimension is
enough to be pressured.

**Windows:**

```
F_target   = profile.free_target_frac × total_bytes            (see §6)
phys       = clamp01((F_target − fast_avail) / F_target)
commit     = clamp01((commit/commit_limit − 0.70) / 0.25)       // 0 at 70%, 1 at 95%
churn_rate = Δchurn_pages / Δt × page_size / total_bytes         // fraction of RAM repurposed per second
churn      = clamp01(churn_rate / 0.002)                          // 0.2% of RAM/s of high-prio repurpose ⇒ 1
P = max(phys, commit, churn)
```

**macOS:**

```
level_floor = {1: 0.0, 2: 0.60, 4: 0.90}[kernel_level]
avail       = clamp01((60 − free_pct) / 60)                       // memorystatus_level 60% ⇒ 0, 0% ⇒ 1
swap_growth = clamp01(Δswap_used / Δt / (64 MiB/s))               // sustained swap-out
P = max(level_floor, 0.8 × avail, swap_growth)
```

The kernel level is authoritative. It only sets a floor, so our own estimate can raise the score
earlier, but never lower it below what the kernel reports.

## 4. Smoothing and state machine

- **EWMA with a time constant**, so that irregular sampling stays correct:
  `α = 1 − exp(−Δt_s / τ)`, with `τ = 20 s`; `S ← S + α (P − S)`. The first sample sets
  `S = P`.
- **States and hysteresis** (on `S`):

| State | Enter when S ≥ | Exit (down) when S < | Min dwell before stepping down |
|---|---|---|---|
| Normal | — | — | — |
| Elevated | 0.40 | 0.30 | 30 s |
| High | 0.65 | 0.55 | 30 s |
| Critical | 0.85 | 0.75 | 60 s |

- **Upward** transitions are immediate (and may skip states).
- **Downward** transitions step one state at a time, and only after the dwell time has passed
  since entering the current state.
- **Emergency override.** A kernel event sets `S = max(S, P_event)` immediately and re-evaluates
  without waiting for the EWMA. The events are Windows `LowMemoryCondition` / commit events, and
  macOS pressure-source `critical`. `P_event` is 0.9.

## 5. Actions, budgets and the ledger

Each action has a **tier**, an **entry state**, a **cooldown** and a **token bucket**:

```text
ActionSpec { id, tier, min_state, cooldown_s, bucket_capacity, refill_per_h, requires: [caps] }
```

An action is *eligible* when:

- `state ≥ min_state`,
- `now − last_run ≥ cooldown_s × penalty`,
- the bucket has at least 1 token,
- the capabilities are present (privileges or helper),
- the platform predicate holds. Predicates are defined per platform: for example, "low-priority
  standby ≥ 256 MiB", or "a candidate idle process exists".

**One action per evaluation**: the lowest eligible tier wins. Higher tiers only run if lower tiers
have been tried in the current pressure episode, or are on cooldown.

### 5.1 Ledger and regret

Every executed action appends a `LedgerEntry` to a ring of **64 entries**:

```text
LedgerEntry { t_ms, action_id, state, fast_avail_before, fast_avail_after,
              baseline_hard_rate, post_hard_pages, freed_pages, refault_ratio, penalty_after }
```

- `baseline_hard_rate` is the EWMA (τ = 120 s) of hard faults per second *before* the action.
- `post_hard_pages = (hard_faults_cum(t+60s) − hard_faults_cum(t)) − baseline_hard_rate × 60`,
  floored at 0.
- `freed_pages = max(0, fast_avail_after − fast_avail_before) / page_size`, measured 2 s after the
  action.
- `refault_ratio = post_hard_pages / max(freed_pages, 1)`.

**Penalty adaptation (per action):**

| Condition | Effect |
|---|---|
| `refault_ratio > 0.25` | `penalty = min(penalty × 2, 16)` |
| `refault_ratio < 0.05` three times in a row | `penalty = max(penalty / 2, 1)` |

The penalty multiplies the cooldown, and **also raises the action's entry threshold**: it
requires `S ≥ enter(min_state) + 0.05 × log2(penalty)`.

This is the closed loop that keeps "aggressive" honest. If purging standby keeps causing disk
reads, the engine purges less often, on its own.

## 6. Profiles (the user's RAM-vs-cache slider)

| Parameter | Conservative | Balanced (default) | Aggressive |
|---|---|---|---|
| Windows `free_target_frac` | 0.03 | 0.06 | 0.10 |
| Minimum tier allowed automatically | 1–2 | 1–4 | 1–5 |
| Standby full-purge bucket (capacity / refill per h) | 1 / 1 | 2 / 4 | 4 / 12 |
| Selective trim candidates idle for ≥ | 30 min | 10 min | 3 min |
| macOS Nudge automatic? | never | never | at High, max 1 per 30 min (helper required) |
| Leak slope threshold | 200 MiB/h | 100 MiB/h | 50 MiB/h |
| Leak growth threshold | max(512 MiB, 40%) | max(256 MiB, 25%) | max(128 MiB, 15%) |

The macOS app is *complementary* by design. Even Aggressive never runs `purge` automatically and
never fakes pressure through allocation.

## 7. Leak detector (normative)

**Input:** a per-process series `(t_h[i], y_mib[i])`, `i = 0..n−1`, oldest first.

1. **Eligibility:**
   - `n ≥ 40`,
   - `t_h[n−1] − t_h[0] ≥ 0.5`,
   - the process is older than 10 min at the first sample used.
2. **Decimation.** If `n > 64`, let `k = ceil(n / 64)` and keep the indices
   `i = n−1, n−1−k, n−1−2k, …` (≥ 0), reordered oldest first. This always keeps the newest sample.
3. **Theil–Sen:**
   - `slope = median{(y_j − y_i)/(t_j − t_i) : i < j, t_j > t_i}`. For an even count, use the mean
     of the two middle values.
   - `intercept = median{y_i − slope·t_i}`.
4. **Mann–Kendall:**
   - `S = Σ_{i<j} sgn(y_j − y_i)`, with `sgn(0) = 0`.
   - `Var = [n(n−1)(2n+5) − Σ_g t_g(t_g−1)(2t_g+5)] / 18` over groups of tied `y` values
     (exact equality).
   - `Z = (S−1)/√Var` if S > 0; `(S+1)/√Var` if S < 0; 0 otherwise.
   - `τ = S / (n(n−1)/2)`.

   All of these are computed on the decimated series, with `n` = the decimated count.
5. **Growth.** `growth = y[last] − y[first]` and `baseline = y[first]` (decimated series).
6. **Plateau check:**
   - Take the last `m = max(8, ceil(n/4))` decimated points and compute their Theil–Sen slope
     `s_tail`.
   - The series is not plateaued if `s_tail ≥ 0.5 × slope`.
7. **Leak decision:** `slope ≥ L_slope && growth ≥ max(L_abs, L_rel × baseline) && τ ≥ 0.6 && Z > 2.33 && !plateau`.
8. **Runaway decision** (independent of the leak rule):
   - On the raw (undecimated) last 2 minutes of samples, the Theil–Sen slope is ≥ 1024 MiB/min
     (= 61,440 MiB/h).
   - At least 5 samples in that window.
9. **ETA:** `eta_h = headroom_mib / slope` when the process is flagged. Headroom is
   `(commit_limit − commit)` on Windows, or the MiB until the Critical threshold on macOS.

**Known-heavy multiplier.** For executables on the built-in heavy list (browsers, IDEs,
hypervisors, games, ML runtimes), use `L_slope × 2` and `L_abs × 2`. The list is data, not code.

## 8. Sampling cadence

| Condition | Interval |
|---|---|
| Popup visible | 1 s (system), 2 s (process list) |
| Idle, state Normal | 10 s system, 30 s processes |
| State ≥ Elevated | 5 s system, 15 s processes |
| Display off / session locked | Paused; kernel events still wake us |
| On battery (optional) | Idle intervals × 2 |

Timers are coalescible:

- Windows: `SetWaitableTimerEx` with a `TolerableDelay` of 10% of the period, minimum 1 s.
- macOS: dispatch timer `leeway` of 10%.

## 9. Determinism and test vectors

- `spec/test-vectors/leak-*.json`: `{ "name", "profile", "heavy": bool, "series": [[t_h, y_mib], …], "expect": { "eligible", "slope", "tau", "z", "plateau", "leak", "runaway" } }`.
- `spec/test-vectors/state-*.json`: `{ "name", "samples": [[t_ms, P, event?], …], "expect_states": ["Normal", …] }`.
- Floating-point comparison: relative `1e-9`, absolute `1e-9`. Decisions must match exactly.
- Regenerate with `python3 spec/reference/policy_ref.py --write spec/test-vectors` (needs no
  dependencies beyond the standard library).
