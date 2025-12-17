# Log Time Machine (Ratatui)

Terminal log viewer focused on fast inspection: tail live logs, pause and scroll without losing lines, see activity spikes, and eventually bookmark/diff periods.

## Current controls
- Quit: `q` / `Ctrl-C`
- Pause/live: `space`, `g`/`End`
- Scroll: `Up`/`Down`/`k`/`j`, `PageUp`/`PageDown`, `Home` to top
- Timeline scrub: `Left`/`Right` to jump across bins; `s`/`S` jump to next/prev spike
- Filters: `/` to enter text (toggle regex with `R`), `F` clears, `1/2/3` toggle INFO/WARN/ERROR, `n/p` next/prev error
- Bookmarks: `b` add, `]`/`[` next/prev
- Diff stub: `A`/`B` set range markers; counts show in status

## Run it
- Mock feed: `cargo run` or `make run-mock`
- Tail a file: `make run-sample` or `cargo run -- --file samples/sample.log`
- Tail stdin: `cat samples/sample.log | cargo run -- --stdin`

## Dev
- `make fmt` / `make check` / `make clippy`
- Code lives in `src/main.rs`; sample log at `samples/sample.log`.

## Roadmap (tracked in GitHub issues)
- Timeline cursor UI polish and bookmark marks over the timeline
- Richer diff view and export
- Test coverage and CI (fmt/clippy)
