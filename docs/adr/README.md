# Architecture decision records

One file per decision that is expensive to reverse or that constrains future work. Copy
[`0000-template.md`](0000-template.md) to the next free number. Status is one of **Proposed**,
**Accepted**, **Superseded by NNNN**. Records are never deleted; a reversed decision gets a new ADR
that supersedes the old one.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-pin-gpui-to-a-zed-revision.md) | Pin GPUI to a Zed revision | Accepted |
| [0002](0002-block-swap-editing-model.md) | Block-swap editing model | Accepted |
| [0003](0003-single-instance-ipc.md) | Single-instance handoff over local IPC | Accepted |
| [0004](0004-startup-budget-and-gate.md) | 50 ms startup budget and the Phase 1 gate | Accepted |
| [0005](0005-incremental-reparse-by-block-windows.md) | Incremental reparse by block windows | Accepted |
| [0006](0006-hot-exit.md) | Hot exit: Quit keeps unsaved documents instead of asking | Accepted |
| [0007](0007-versions-and-release-channels.md) | Versions and release channels | Accepted |
