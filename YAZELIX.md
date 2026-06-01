# Yazelix Helix Fork Boundary

This repository tracks Helix Steel and carries the smallest Yazelix-specific
runtime hooks needed by managed Yazelix sessions.

Current Yazelix-owned fork delta:

- `hx --config-dir <path>` for self-contained managed Helix config lookup
- optional local Helix action bridge, enabled only when
  `YAZELIX_HELIX_BRIDGE=1`

The bridge is not a general remote-control surface for arbitrary Helix
instances. It is a Yazelix-managed local IPC endpoint used to replace terminal
keystroke injection for editor-owned actions.

Bridge startup requires:

- `YAZELIX_STATE_DIR`
- `YAZELIX_HELIX_BRIDGE_SESSION_ID`
- `YAZELIX_HELIX_BRIDGE_AUTH_TOKEN`

Optional context:

- `YAZELIX_HELIX_BRIDGE_INSTANCE_ID`
- `YAZELIX_HELIX_MANAGED_CONFIG_PATH`
- `ZELLIJ_SESSION_NAME`
- `ZELLIJ_TAB_POSITION`
- `ZELLIJ_PANE_ID`

When enabled, Helix writes bridge registry and token files below:

```text
$YAZELIX_STATE_DIR/helix_bridge/<session_id>/
```

The registry advertises the native local IPC transport for the current
platform: Unix sockets on Unix-like systems and best-effort named pipes on
native Windows.

Supported first-slice actions:

- `helix.get_context`
- `helix.set_cwd`
- `helix.open_directory`
- `helix.open_files`

Zellij remains responsible for panes, tabs, focus, layout, and workspace
routing. The bridge owns only editor-local actions after the target Helix
instance has been selected.
