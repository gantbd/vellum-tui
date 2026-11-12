# Vellum TUI

`vellum-tui` is the standalone terminal application and reusable Ratatui
presentation library for Vellum. It is a peer of the macOS GUI and MCP
projection, not part of the Vellum engine and not a second source/history
authority.

The executable may start empty, from files, from directories, or from a mix:

```sh
cargo run --locked --
cargo run --locked -- README.md src
```

It can instead project a running Vellum service without taking source or
persistence authority:

```sh
cargo run --locked -- --service /path/to/vellum.sock
cargo run --locked -- --service /path/to/vellum.sock --credential /path/to/credential
```

In the empty state, `Ctrl-O` adds a file or directory and `Ctrl-Q` exits. In an
active editor, `Ctrl-O` adds another bounded root. Directory discovery is
deterministic, refuses symbolic links and special files, and admits at most 16
unique regular files. Run `vellum-tui --help` for the complete current key map
and qualification limits. In service mode, the `Ctrl-O` prompt accepts `f PATH`
or `d PATH`; edits and saves use witnessed service operations, and an
indeterminate operation must be replayed exactly before exit.

`tests/live/service_journey.sh` builds the exact sibling Vellum daemon, starts
it on private Unix sockets with edit/save ceilings, drives the reusable TUI
controller through open/edit/save/disconnect, verifies exact persisted bytes,
and requests orderly control-socket shutdown. CI runs this journey without
opening an interactive terminal.

## Composition boundary

The `vellum_tui` library owns terminal projection, key translation, bounded
prompts, and empty-state rendering. It owns no terminal lifecycle, filesystem
authority, document source, or history. The `vellum-tui` binary is the
composition root that selects Vellum's macOS adapters and Ratatui lifecycle.
`draw_in`, `draw_empty_in`, and `draw_comments_in` render only inside an
assigned `Rect`; their full-frame counterparts remain convenience wrappers for
the standalone application.

A future workbench can import the library beside Agora or another engine's
Ratatui library under one executable. It must provide the shared event loop and
route effects to each component's own authority; it must not import this
standalone application shell or merge engine state.

## Current source checkout

Until the pinned Vellum commit is reachable from its declared GitHub remote,
this unreleased peer uses a sibling `../vellum` path for coordinated local
qualification:

```text
parent/
  vellum/
  vellum-tui/
```

Build with `cargo test --locked --offline`. GitHub source installation will
replace those paths with exact Vellum Git revisions and is not claimed yet.
Nothing is published to crates.io.

## Platform and nonclaims

The current reference application is macOS-only. Its Unix-socket service mode
is a local peer projection, not a remote-network transport claim. It does not
yet claim live filesystem watching, IME preedit, bidi layout, screen-reader
behavior, release signing, notarization, or a stable UI API.
