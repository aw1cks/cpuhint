# Fuzzing

The fuzz project is intentionally separate from the production Cargo workspace
so `libfuzzer-sys` and its build dependencies cannot enter the cpuhint preload
DSO.

Install `cargo-fuzz`, then run both targets with nightly Rust:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run parse_cpu_list -- -max_total_time=60
cargo +nightly fuzz run build_contiguous_mask -- -max_total_time=60
```

CI smoke runs can use a shorter bound such as `-max_total_time=15`. Longer
scheduled jobs should retain the generated corpus but must not commit crash
artifacts until they have been reviewed and minimized.

Generated corpus entries are ignored locally; the small named parser seeds are
tracked so a fresh checkout starts with valid contiguous and disjoint inputs.

The parser seeds are literal CPU-list files:

| Seed | Meaning |
| --- | --- |
| `single` | One virtual CPU, `0` |
| `contiguous` | Four contiguous CPUs, `0-3` |
| `disjoint` | Eight non-contiguous CPUs, `0-3,8-11` |

The mask target uses a binary input format rather than a text corpus. Byte zero
selects the output buffer length. Bytes one through eight, interpreted as a
little-endian `u64`, select the requested CPU count. This deliberately drives
zero-length, undersized, aligned, unaligned, and oversized cases.

Useful investigation commands:

```sh
cargo +nightly fuzz list
cargo +nightly fuzz run parse_cpu_list -- -print_final_stats=1
cargo +nightly fuzz cmin parse_cpu_list
cargo +nightly fuzz tmin parse_cpu_list fuzz/artifacts/parse_cpu_list/<artifact>
```

If a crash is found, preserve the artifact, reproduce it with the printed
command, minimize it, and add a focused unit or regression test before deleting
the artifact.
