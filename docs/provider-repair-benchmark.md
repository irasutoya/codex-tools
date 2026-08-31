# Provider Repair Resource Benchmark

This benchmark exercises the complete rollout-repair path, including preflight,
backups, guarded writes, verification, and manifest generation. The fixtures
are deterministic and contain no user data. Database and paginated-history
repair are covered by unit tests but are not part of these resource fixtures.

## Scenarios

- `large`: 100 rollout files, approximately 4 MiB each.
- `small`: 10,000 rollout files, approximately 1 KiB each.

Fixture generation runs outside the measured process. Peak thread count is
sampled every 10 ms, so very short-lived threads can be missed. RSS is the
macOS `/usr/bin/time -l` process high-water mark.

## Results

Measured on an Apple `Mac17,9` with 18 logical CPUs, macOS 26.6.2, and Rust
1.98.0. Values are single warm-cache runs and should be used as resource-shape
evidence rather than a stable throughput target.

| Scenario | Implementation | Elapsed | Peak RSS | Workers created | Max sampled process threads |
| --- | --- | ---: | ---: | ---: | ---: |
| 100 large | Unbounded, retained buffers | 3.11 s | 1,711,390,720 B | 100 | 100 |
| 100 large | 4 workers, hash-only plans | 7.17 s | 47,120,384 B | 4 | 7 |
| 10,000 small | Unbounded, retained buffers | 43.06 s | 219,807,744 B | 10,000 | 11 |
| 10,000 small | 4 workers, hash-only plans | 42.77 s | 48,087,040 B | 4 | 7 |

The large-file scenario reduces peak RSS by 97%. It deliberately trades some
elapsed time for a second guarded per-file analysis during commit, preserving
the all-files preflight while retaining only hashes and counters. The small-file
scenario no longer creates one OS thread per rollout. “Workers created” follows
the worker construction in the measured source; the separate sampled process
count includes the test harness and can miss short-lived baseline threads.

The baseline used `provider_sync.rs` from commit
`73b5c80a57de18c3e53f98f8082b14aa751399d8`, compiled with the same dirty
worktree dependencies and release profile as the optimized implementation. The
preserved baseline executable had SHA-256
`553be256f407069fa036b16fd45b25ba849975d6f0d57e74ac25cd1a96d97d8e`.
The table records the raw `/usr/bin/time -l` RSS byte values; the 97% reduction
is `(1711390720 - 47120384) / 1711390720`, rounded to the nearest percent.

## Reproduce

Build the optimized library test executable:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --release --locked --lib --no-run
```

Use the executable path printed by Cargo:

```sh
scripts/measure-provider-repair-macos.sh <lib-test-binary> large
scripts/measure-provider-repair-macos.sh <lib-test-binary> small
```

The runner creates and removes isolated temporary fixture directories. It does
not read or modify the real Codex home directory.
