# Vellum TUI repository roadmap

`vellum-tui` remains a reusable editor presentation library. The standalone
`vellum-tui-cli` package and Workbench are separate composition roots.

The standalone CLI package now has an explicit service-backed headless
inventory command with a versioned `workspace.inventory.v1` JSON result. It
uses the workspace client directly, constructs no
terminal or editor cursor state, and passes in the real-daemon journey before
the reusable editor controller opens, edits, saves, and disconnects.
The peer publishes a closed interactive/headless presentation capability
manifest for pre-connection host negotiation. It conveys no edit, save, source,
or persistence authority.
Standalone headless failures retain the versioned envelope and distinguish
configuration/transport/connect refusal, not-sent access, outcome-unknown
delivery, semantic refusal, invalid projection, and uncertain disconnect.

- Keep argument parsing, terminal lifecycle, daemon startup, adapter selection,
  global shortcuts, and process exit outside the reusable component.
- Preserve the standalone non-headless editor by having Vellum's CLI host this
  library with its selected document/workspace service and platform adapters.
- Give the same CLI bounded headless document/workspace operations that do not
  create selection, cursor, focus, or other presentation state.
- Let Workbench import the library and either attach to Vellum or host its
  service runtime under one composite executable and lifecycle.
- Qualify standalone interactive/headless and Workbench interactive/headless
  paths with identical revision, dirty-state, persistence, authorization,
  durable-operation lookup, and outcome-unknown behavior.

Neither host may acquire source, history, persistence, or edit authority from
the TUI component.
