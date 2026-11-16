# Vellum TUI

`vellum-tui` is the standalone terminal application and reusable Ratatui
presentation library for Vellum. It is a peer of the macOS GUI and MCP
projection, not part of the Vellum engine and not a second source/history
authority.

The executable may start empty, from files, from directories, or from a mix:

```sh
cargo run -p vellum-tui-cli --bin vellum-tui --locked --
cargo run -p vellum-tui-cli --bin vellum-tui --locked -- README.md src
```

It can instead project a running Vellum service without taking source or
persistence authority:

```sh
cargo run -p vellum-tui-cli --bin vellum-tui --locked -- --service /path/to/vellum.sock
cargo run -p vellum-tui-cli --bin vellum-tui --locked -- --service /path/to/vellum.sock --credential /path/to/credential
cargo run -p vellum-tui-cli --bin vellum-tui --locked -- headless --service /path/to/vellum.sock
cargo run -p vellum-tui-cli --bin vellum-tui --locked -- capabilities
```

Headless emits a versioned `workspace.inventory.v1` JSON observation with
current bounded document/root counts and constructs no editor, focus,
selection, or cursor state.
The `capabilities` command prints Vellum's peer-owned composition manifest
without connecting. Interactive composition advertises `workspace.edit.v1` and
`workspace.save.v1`, but those identifiers grant no edit or persistence
authority; the connected service still enforces its independent action ceiling.

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
deliberately discards one accepted save reply, requires the client projection to
become indeterminate, and proves only the exact retained save can resolve it.
The gate then requests orderly control-socket shutdown. CI runs this journey
without opening an interactive terminal.

## Composition boundary

The `vellum_tui` library package owns terminal projection, key translation, bounded
prompts, and empty-state rendering. It owns no terminal lifecycle, filesystem
authority, document source, or history. The separate `vellum-tui-cli` package
owns the `vellum-tui` binary and selects Vellum's macOS adapters and Ratatui lifecycle.
`draw_in`, `draw_empty_in`, and `draw_comments_in` render only inside an
assigned `Rect`; their full-frame counterparts remain convenience wrappers for
the standalone application.

A future workbench can import the library beside Agora or another engine's
Ratatui library under one executable. It must provide the shared event loop and
route effects to each component's own authority; it must not import this
standalone application shell or merge engine state.

## Current source checkout

The pinned Vellum and Keystone commits are reachable from their declared GitHub
remotes. This unreleased peer deliberately keeps a sibling `../vellum` path for
coordinated source qualification:

```text
parent/
  vellum/
  vellum-tui/
```

Build the reusable presentation and separate standalone CLI with
`cargo test --workspace --all-targets --locked --offline`.
Standalone Git source installation is not claimed yet. Nothing is published to
crates.io.

## Platform and nonclaims

The current reference application is macOS-only. Its Unix-socket service mode
is a local peer projection, not a remote-network transport claim. It does not
yet claim live filesystem watching, IME preedit, bidi layout, screen-reader
behavior, release signing, notarization, or a stable UI API.
