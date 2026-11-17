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
`workspace.save.v1`; interactive and headless composition also advertise the
selected `workspace.durable-save.execute.v1` and
`workspace.durable-save.lookup.v1` profile. Those identifiers grant no edit or
persistence authority; the connected service still enforces authentication,
its independent action ceiling, and the selected filesystem adapter.

Use `vellum-tui durable-save-execute SOCKET CREDENTIAL
vdo1.OPERATION_HEX DOCUMENT_ORDINAL EXPECTED_REVISION` after persisting the
operation identity, and use the corresponding `durable-save-lookup` command to
recover only that exact historical receipt. Transport uncertainty is reported
without minting or retrying a different operation. Save As submission, receipt
retirement, arbitrary destinations, and broader durable effects are not exposed
by this profile.

In the empty state, `Ctrl-O` adds a file or directory and `Ctrl-Q` exits. In an
active editor, `Ctrl-O` adds another bounded root. Directory discovery is
deterministic, refuses symbolic links and special files, and admits at most 16
unique regular files. Run `vellum-tui --help` for the complete current key map
and qualification limits. In service mode, the `Ctrl-O` prompt accepts `f PATH`
or `d PATH`; edits and saves use witnessed service operations, and an
indeterminate operation must be replayed exactly before exit.

The reusable service controller also retains cursor, selection, and persistence
truth independently for every admitted document. Hosts may mark a sparse
workset of 1–16 documents; with no marks, the focused document is the explicit
one-item fallback. `DurableSaveBatchComponent` re-observes the exact revision of
every selected document, executes each authenticated save on its own fresh
connection, and retains a separate receipt or refusal per child. Persistence is
deliberately sequential and non-atomic: an unknown child stops the run, later
children remain visibly not sent, exact receipt lookup is required, and
continuation requires another explicit arm/confirm.

For changes that should publish together in memory, `AtomicReplaceComponent`
adapts marked-file query/replace into Vellum's stronger owner transaction. It
accepts bounded literal find/replacement fields, re-observes all 1–16 marked
documents, previews match counts and exact revisions, then arms one
`WorkspaceEdit` request. The owner either publishes the complete revision-bound
set under one generation or refuses it; an uncertain reply is recovered only by
replaying that exact retained request. The returned snapshots become dirty
editor state. This is intentionally not a multi-file filesystem transaction,
so durable persistence remains the independently receipted workflow above.

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
