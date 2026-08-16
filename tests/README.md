# Linux Preload Smoke Test

This procedure validates the ELF preload DSO against the live Linux and LXCFS
environment. It is not a substitute for the release ABI gate in the root
`README.md` or the compatibility build requirements in `DESIGN.md`.

Build the library and probe on the target Linux image:

```sh
cargo build --release -p cpuhint-preload
cc -O2 -Wall -Wextra -Werror -o preload-probe tests/preload_probe.c
```

With an LXCFS CPU view of `0-1`, run the target-built library explicitly:

```sh
LD_PRELOAD="$PWD/target/release/libcpuhint_preload.so" ./preload-probe 2
```

The probe verifies all four interposed CPU count APIs, the zero-PID synthetic
affinity mask, cleared high bits, preserved `errno` on affinity success, and an
unrelated `sysconf` forward.

Inspect the linked artifact before using it outside this smoke test:

```sh
readelf -d target/release/libcpuhint_preload.so
readelf --version-info target/release/libcpuhint_preload.so
nm -D --defined-only target/release/libcpuhint_preload.so
nm -D --undefined-only target/release/libcpuhint_preload.so
readelf -W -S target/release/libcpuhint_preload.so
```

A native build is suitable only for an initial ABI smoke test. Release artifacts
must be built in a controlled compatibility environment and checked against the
project's declared glibc support floor.

The native smoke artifact must not have an undefined or dynamically exported
`rust_eh_personality`, or an `.init_array` section. The release inspection also
rejects these artifacts. `.eh_frame` may remain for stack walking; panic paths
still terminate through the crate's aborting panic handler.

The probe must run in the container whose expected CPU count is supplied. A
host or unrelated container with a different LXCFS view will correctly fail the
probe when given the wrong expected count.
