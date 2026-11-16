# Changelog

## Unreleased

- Qualified the interactive `workspace.edit.v1` and `workspace.save.v1`
  composition capabilities against the real daemon, including exact persisted
  bytes and an accepted save reply deliberately discarded before exact replay
  resolves the indeterminate projection.
- Recovered the mature Ratatui projection from Vellum history as an independent
  composable peer.
- Added lawful empty startup and bounded file/directory root admission.
- Removed the historical optional desktop binary; the macOS GUI is a separate
  peer repository.
- Added an explicit service-backed headless inventory path that reuses the
  workspace client without constructing terminal, focus, selection, or cursor
  state, and qualified it alongside the real-daemon editor journey.
- Extracted the standalone `vellum-tui` executable into the sibling
  `vellum-tui-cli` package, leaving the root package as the reusable
  presentation component and keeping concrete platform adapters at the CLI
  composition boundary.
- Versioned the standalone headless inventory as a machine-readable
  `workspace.inventory.v1` JSON observation.
