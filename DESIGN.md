# cpuhint Design

## Problem

Kubernetes commonly runs overprovisioned pods on large shared nodes. A CPU
limit controls CPU time through cgroups but normally does not narrow a process's
kernel affinity mask. Consequently, an application limited to a small fraction
of a node can still discover every host CPU and create an oversized worker pool.
The resulting threads and processes increase memory use, context switching, and
CFS throttling.

LXCFS can provide container-aware files such as `/proc/cpuinfo`, `/proc/stat`,
and `/sys/devices/system/cpu/online`. It cannot change the result of system
calls such as `sched_getaffinity(2)`.

### Cgroup Controls Are Independent

CPU request, quota, weight, and cpuset are not interchangeable forms of a
single CPU allocation. A request affects placement and, depending on the
orchestrator, relative contention entitlement. A cgroup v2 `cpu.max` value
limits average CPU time across periods. A cpuset limits which logical CPUs a
task may execute on. CPU weight only affects relative allocation among runnable
sibling cgroups; it cannot be converted into an absolute CPU count.

This distinction explains the discovery mismatch. A pod with `cpu.max` equal to
two CPUs may retain a broad task affinity mask. A call to
`sched_getaffinity(0, ...)` therefore counts the node-visible CPU set, not the
pod's quota. Similarly, host-oriented libc CPU count functions need not reflect
the cgroup bandwidth limit. The kernel behavior is correct: quota governs
runtime, whereas affinity governs eligible placement.

For applications that size concurrency from CPU discovery, the distinction is
often unhelpful. A worker pool sized from a 64-CPU affinity mask in a two-CPU
quota cgroup can create dozens of runnable workers that receive only two CPUs
of aggregate sustained runtime. The cost is generally more memory, scheduling
overhead, contention, and quota throttling rather than more useful work.

### Representative Consumers

The initial scope targets dynamically linked programs using ordinary glibc
entry points, including code equivalent to:

```c
long count = sysconf(_SC_NPROCESSORS_ONLN);
int count = get_nprocs();
```

or affinity-counting code:

```c
cpu_set_t mask;
sched_getaffinity(0, sizeof(mask), &mask);
int count = CPU_COUNT(&mask);
```

These patterns occur in generic C/C++ worker pools and can appear below
language or tool APIs such as Python CPU discovery, Node.js CPU enumeration,
native extensions, compilers, and build systems. Runtime behavior varies by
release: some JVM, Go, Rust, and JavaScript runtime versions inspect cgroups
directly or use direct syscalls and can bypass cpuhint. Explicit runtime worker
limits remain the preferred control where available.

## Goal

cpuhint is a small Linux preload library that aligns selected glibc CPU
discovery APIs with the LXCFS CPU view. It is a concurrency hint for software
that sizes work from CPU count. It is not CPU resource control, topology
virtualization, a security boundary, or a replacement for cgroups.

## Architecture

```text
cpuhint-core       no_std CPU-list parsing and virtual-mask construction
cpuhint-linux      raw Linux access to the LXCFS online CPU file
cpuhint-preload    no_std glibc ABI interposition DSO
cpuhint-diagnose   normal diagnostic executable using the same resolver
```

`cpuhint-core` has no filesystem, libc, or process-global dependencies.
`cpuhint-linux` reads the canonical view through rustix's Linux raw backend.
`cpuhint-preload` owns the narrow unsafe C ABI boundary. The production DSO
avoids allocation, locks, logging, constructors, thread-local state, and Rust
unwinding across its C ABI.

## Canonical View

On every interposed call, cpuhint opens, reads, parses, and closes:

```text
/sys/devices/system/cpu/online
```

The file uses Linux CPU-list syntax, for example `0`, `0-3`, or `0-3,8-11`.
The parser accepts only nonempty, ascending, ordered, non-overlapping lists
with checked arithmetic and ABI-sized counts. It records both CPU count and
whether the list is exactly contiguous from zero.

Reading LXCFS rather than independently calculating cgroup quota means cpuhint
inherits LXCFS quota rounding, hierarchy handling, and cpuset clamping. If the
view cannot be read or parsed, cpuhint fails open.

## Interposed Functions

| Function | Behavior when LXCFS view is valid |
| --- | --- |
| `sysconf(_SC_NPROCESSORS_CONF)` | Return LXCFS CPU count |
| `sysconf(_SC_NPROCESSORS_ONLN)` | Return LXCFS CPU count |
| `get_nprocs()` | Return LXCFS CPU count |
| `get_nprocs_conf()` | Return LXCFS CPU count |
| `sched_getaffinity(0, ...)` | Return a contiguous virtual mask when the LXCFS list is contiguous from zero |

All other `sysconf` names and all count failures forward to real glibc symbols
resolved with `RTLD_NEXT`. Calls to `sched_getaffinity` for a nonzero PID or TID
pass through unchanged.

For zero-PID affinity, cpuhint first invokes the raw kernel syscall. It
preserves real failures, normalizes the successful kernel result like glibc,
then clamps the synthetic count to the real affinity count. Synthetic bits are
only used for an LXCFS view `0..N-1`; a non-contiguous LXCFS list retains the
real mask.

## Safety And Failure Behavior

cpuhint preserves `errno` on successful hooks and sets it only for failures it
every hook passes through unchanged.

| Condition | Behavior |
| --- | --- |
| Missing, inaccessible, oversized, or malformed LXCFS view | Real result |
| Empty CPU view | Real result |
| Non-contiguous online list | Count APIs use count; affinity stays real |
| Real affinity failure | Original return value and `errno` |
| LXCFS count exceeds real affinity count | Synthetic result is clamped |
| Missing real forwarding symbol | `-1` and `ENOSYS` |

## Limits

The virtual affinity mask is not an enforced kernel affinity. It can disagree
with `/proc/self/status`, `sched_getcpu`, topology and NUMA APIs, direct
syscalls, and affinity setters. Programs that bind threads or make NUMA
placement decisions should use a real restricted cpuset instead.

cpuhint does not cover static binaries, direct or inline syscalls, every libc,
or every runtime. Runtime-specific controls remain useful complements.

Unlimited CPU quota with a broad shared cpuset is intentionally not solved by
inventing a count from CPU weight, a scheduler request, or an environment
variable. That policy needs a real restricted cpuset or a trusted advertised
parallelism source shared by LXCFS and cpuhint.

## Testing

The core parser and mask builder have unit tests and libFuzzer targets. The C
smoke probe verifies public hook behavior, exact virtual affinity bits,
passthrough behavior, and `errno` preservation against a live LXCFS mount.

Every release artifact must be checked for architecture, intended exports,
unexpected dependencies, unsupported glibc versions, Rust runtime symbols,
constructors, and ELF hardening properties. Test all supported architectures,
glibc baselines, and representative pod images before global preload rollout.
