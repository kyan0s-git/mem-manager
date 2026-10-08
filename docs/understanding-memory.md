# Why "ready for apps" counts the cache

MemManager's dashboard leads with how much memory is **ready for apps**, not how much is "used".
This page explains why. If you prefer the operating system's own terms, switch
**Settings → Values** to *Windows standard* or *Activity Monitor terms*. Every number in both views
is a real value from the OS.

## Memory that isn't "in use" isn't wasted. It's cache

Both Windows and macOS keep files and app data you used recently in otherwise-idle memory:

- Windows calls it the **standby list**.
- macOS calls it **Cached Files**.

When you open the file again, it comes from memory instead of the disk.

That cache isn't taking anything away from your apps. The moment an app needs memory, the OS
hands cached memory over instantly, without writing anything to disk. So memory that is cached is
**ready for apps**, just like free memory, with a bonus: until an app claims it, it speeds things
up.

That's why a healthy computer that has been on for a while often shows 70–90% "used" in the
classic view. It's doing its job.

## Emptying the cache makes things slower, not faster

We measured it ([benchmarks](benchmarks.md)):

| | Freed | Afterwards |
|---|---|---|
| Windows: clearing the standby cache | 5.8–8.0 GB | Re-reading a file took **19–26× longer** |
| macOS: purging the disk cache | 2.7–3.1 GB | Re-reading a file took **3.6–11× longer** |

The memory was "freed", but it was memory the OS would have handed to an app instantly anyway.
The only thing that changed is that the next thing you open has to come from the disk.

## What actually gives you more room

**Apps** hold memory that can't simply be handed out, so that's where MemManager focuses:

- **Idle apps holding a lot of memory.** MemManager shows them, for example "Slack has been idle
  for 3 h and holds 1.4 GB". Quitting one frees that memory for real.
- **Leaking apps** grow without limit. MemManager detects the trend and offers to restart the
  app, which returns all of it.
- **Under real pressure**, when apps start pushing each other out, MemManager steps in on its own.
  On Windows it trims idle apps and clears low-value cache. On macOS it asks apps to release
  caches they can rebuild. Each action's cost is measured afterwards, and actions that hurt are
  backed off.

## Reading the dashboard

- **Ready for apps:** free memory plus cache. This is the number to watch. As you open more apps,
  the cache shrinks to make room, and the dashboard says so ("Cache made room for 1.2 GB of app
  memory…").
- **Apps / Apps & macOS:** memory that apps and the system are actually using.
- **Speed-up cache:** recently used data kept in memory, handed to apps on demand.
- **Comfortable → Busy → Tight → Critical:** the real memory-pressure state, from how hard the OS
  is working to find memory, not from a percentage.

If memory really is short, the state changes and MemManager tells you. The warnings work the same
in both views.
