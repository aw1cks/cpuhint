# cpuhint

cpuhint is a small Linux preload shim for overprovisioned container pods. It
aligns selected glibc CPU-discovery calls with the CPU view provided by LXCFS,
helping applications size worker pools from their container-visible CPU count
rather than the host CPU count.

It is a hint, not a resource-control mechanism. Cgroups, CPU quota, cpusets,
and the kernel scheduler remain authoritative.

## Why This Exists

Container CPU controls answer different questions:

| Control | What it changes |
| --- | --- |
| CPU request | Scheduler admission and relative contention entitlement |
| CPU limit / cgroup `cpu.max` | Maximum average CPU time over a CFS period |
| cpuset | Logical CPUs on which a task may run |

A CPU limit is a bandwidth limit, not an affinity limit. A pod limited to two
CPUs can still have an affinity mask covering dozens or hundreds of CPUs on its
node. It may run on any of those CPUs, but after consuming its quota it is
throttled until more runtime becomes available.

Many applications discover CPU count once during startup and use it as a worker
count. In an overprovisioned pod, this can turn a modest CPU entitlement into a
large thread pool. The extra workers compete for a small quota, consume stack
and runtime memory, create context switching, and can increase throttling.

For example, a two-CPU-limited pod on a 64-CPU node may observe:

```text
cgroup cpu.max:                 200000 100000
kernel affinity:                 CPUs 0-63
LXCFS online CPUs:               0-1
```

Without a process-visible CPU view, a program that sees the kernel affinity or
the host count may select 64 workers even though it receives only two CPUs of
sustained runtime. cpuhint makes the selected discovery calls agree with the
LXCFS view of two CPUs.

## How It Works

cpuhint reads LXCFS's `/sys/devices/system/cpu/online` view on each call and
interposes these dynamically linked glibc APIs:

- `sched_getaffinity` for the calling thread only (`pid == 0`)
- `sysconf` for `_SC_NPROCESSORS_CONF` and `_SC_NPROCESSORS_ONLN`
- `get_nprocs`
- `get_nprocs_conf`

For a contiguous LXCFS view such as `0-1`, count APIs return `2` and affinity
returns virtual bits zero and one. Missing, malformed, or unsupported views
fail open to real libc and kernel behavior.

See [`DESIGN.md`](DESIGN.md) for the complete behavior, safety model, and
limitations.

## Typical Discovery Paths

cpuhint targets applications that use standard dynamically linked glibc APIs
to decide how much parallel work to create. Examples include:

```c
long workers = sysconf(_SC_NPROCESSORS_ONLN);
```

```c
#define _GNU_SOURCE
#include <sched.h>

cpu_set_t mask;
sched_getaffinity(0, sizeof(mask), &mask);
int workers = CPU_COUNT(&mask);
```

```c
#include <sys/sysinfo.h>

int workers = get_nprocs();
```

Common higher-level paths that can eventually use these interfaces include C
and C++ worker pools, Python's `os.cpu_count()` or `os.sched_getaffinity(0)`,
Node.js native CPU discovery, and language runtimes or build tools that use
glibc CPU-count helpers. Exact runtime behavior varies by version: modern
runtimes may inspect cgroups directly, issue direct syscalls, or use their own
configuration instead.

The shim does not replace explicit application settings. Prefer a supported
runtime option when one exists, such as `GOMAXPROCS` for Go,
`-XX:ActiveProcessorCount` for the JVM, or application-specific worker limits.
Those settings are also the required fallback for static binaries and direct
syscall users.

## Crates

| Crate | Purpose |
| --- | --- |
| `cpuhint-core` | `no_std` CPU-list parser and mask builder |
| `cpuhint-linux` | Raw Linux LXCFS resolver |
| `cpuhint-preload` | Linux/glibc preload `cdylib` |
| `cpuhint-diagnose` | Standalone diagnostic executable |

## Development

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Build the diagnostic executable:

```sh
cargo run -p cpuhint-diagnose
```

Build the Linux DSO on Linux:

```sh
cargo build --release -p cpuhint-preload
```

The artifact is `target/release/libcpuhint_preload.so`.

## Smoke Test

On a Linux pod with LXCFS, first inspect the online CPU view:

```sh
cat /sys/devices/system/cpu/online
```

Build the C probe and run it with the expected count. For `0-1`:

```sh
cc -O2 -Wall -Wextra -Werror -o preload-probe tests/preload_probe.c
LD_PRELOAD="$PWD/target/release/libcpuhint_preload.so" ./preload-probe 2
```

The probe checks every hook, exact synthetic mask bits, nonzero-PID affinity
passthrough, unrelated `sysconf` forwarding, and `errno` preservation. See
[`tests/README.md`](tests/README.md) for artifact inspection requirements.

## Deployment

Test with `LD_PRELOAD` before installing globally. A production image can add
an absolute, root-owned path to `/etc/ld.so.preload`, one library path per line:

```text
/usr/lib/libcpuhint_preload.so
```

This affects nearly every dynamically linked process. Canary by image and
architecture, preserve existing preload entries, and retain an external rollback
path. Validate artifact dependencies and glibc symbol versions against the
project's supported compatibility floor before deployment.

## Fuzzing

The separate [`fuzz/`](fuzz/) project fuzzes CPU-list parsing and mask-building
invariants without adding fuzzing dependencies to the production DSO:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run parse_cpu_list -- -max_total_time=60
cargo +nightly fuzz run build_contiguous_mask -- -max_total_time=60
```

## Releases

GitHub Actions runs formatting, tests, and clippy for pull requests and pushes
to `main`. A separate fuzz workflow runs every Monday at 03:17 UTC and can be
started manually from the Actions tab. Fuzz runs are bounded; longer fuzzing
should run separately and retain its generated corpus locally.

Release Please runs on pushes to `main`. It reads Conventional Commit messages,
opens or updates a release pull request, updates `VERSION` and `CHANGELOG.md`,
then creates a GitHub source release and tag after that pull request is merged.

Use Conventional Commit prefixes so the generated release follows semantic
versioning:

| Commit | Release effect |
| --- | --- |
| `fix: preserve errno on resolver fallback` | Patch release |
| `feat: add a diagnostic affinity report` | Minor release |
| `feat!: change the preload ABI` | Major release |
| `docs: clarify cpuset behavior` | No release by default |

Release notes include feature, fix, performance, revert, and maintenance
commits. Renovate uses `chore(deps)`, so dependency updates appear in the
Maintenance section when a release is already being created by a releasable
change. Maintenance commits alone do not create a version bump.

The workflows assume `main` is the default branch. For repositories that use a
different default branch, change both workflow triggers. The repository setting
that allows GitHub Actions to create pull requests must also be enabled.

Release Please creates a GitHub tag and source release with generated notes. It
does **not** compile or attach binary assets. Do not attach a locally built
preload DSO as a public asset: build release binaries in a controlled
compatibility environment, verify their ELF dependencies and symbol versions,
and add a dedicated artifact workflow when that release process is available.

## Non-Goals

- Enforcing CPU limits or affinity.
- Complete CPU, topology, NUMA, procfs, or sysfs virtualization.
- Supporting static executables or direct syscalls.
- Replacing explicit runtime worker-count settings.
- Deriving an advertised CPU count from CPU weight or an untrusted environment
  variable.
