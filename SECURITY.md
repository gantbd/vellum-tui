# Security policy

This repository is unreleased. Report suspected vulnerabilities privately to
the repository owner rather than opening a public issue containing workspace
paths or source text.

The TUI is a trusted local composition. Its relevant boundaries are terminal
input/output, literal startup and prompt locators, local file adapters, AppKit
clipboard access, annotation files, and the Vellum host contracts. Workspace
paths and source are private content and must not enter ordinary diagnostics or
telemetry.

Root admission is bounded and fail-closed: descriptor-anchored discovery
refuses symlinks and special files, plans no partial editor mutation, and the
editor host independently revalidates locators when opening them. This is not a
sandbox against a malicious same-user process or hostile parent directory.

Before a release, review STRIDE and LINDDUN against the exact composition,
qualify terminal escape/control handling, dependency provenance, clipboard and
persistence uncertainty, and publish an authenticated reporting channel.
