# Architecture

This document explains how Kubiru is put together, so you can find your way
around before changing it.

## Overview

```
┌──────────────────────────── kubiru (desktop app) ────────────────────────────┐
│                                                                              │
│  main.rs ── Workspace ─┬─ Sidebar / top bar / namespace switcher             │
│                        ├─ OverviewPage                                       │
│                        ├─ ResourcesPage ── ResourceTable (TableDelegate)     │
│                        ├─ ManifestPage  ── EditorState (YAML/JSON)           │
│                        ├─ ClustersPage  ── ClusterForm (dialog)              │
│                        └─ SettingsPage                                       │
│                                                                              │
│  Session: clusters, active cluster, KubeClient, namespace, epoch             │
└───────────────┬──────────────────────────────────────────────────────────────┘
                │ background threads
┌───────────────▼──────────── kubiru-core (no UI) ─────────────────────────────┐
│  Registry ─┬─ Store (SQLite: clusters, preferences)                          │
│            └─ TokenVault (OS keychain; in-memory for tests)                  │
│  kube::KubeClient ── blocking HTTPS (reqwest + rustls)                       │
│  kube::table / manifest / overview ── pure functions over serde_json::Value  │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Crates

### `kubiru-core`

Everything that can be tested without a window lives here. It has no
dependency on GPUI.

| Module | Responsibility |
| --- | --- |
| `secret` | `Secret`, a string with redacted `Debug` output that is zeroed on drop. |
| `cluster` | Connection settings (`ClusterDraft` → `ValidCluster`) and their validation. All problems are reported together, keyed by form field. |
| `store` | SQLite persistence with versioned migrations (`PRAGMA user_version`). Holds clusters and preferences, never tokens. |
| `vault` | `TokenVault` trait, implemented by `KeyringVault` (the OS keychain) and `MemoryVault` (tests). |
| `registry` | `Registry` keeps a cluster's database row and keychain entry consistent and is safe to share between threads. |
| `kube::resource` | The browsable kinds, their API paths, and Kubernetes name validation. |
| `kube::client` | `KubeClient`: list, get, replace (PUT) and delete, plus `verify` for connection tests. |
| `kube::table` | Turns API objects into rows with kubectl's columns, statuses and sort keys. |
| `kube::manifest` | YAML/JSON rendering and parsing, and `prepare_update`, which guards edits. |
| `kube::overview` | Aggregates nodes, deployments and pods for the overview page. |

### `kubiru`

The GPUI Kit application. Each page is a GPUI entity that renders itself and
emits events. The `Workspace` owns the pages and handles navigation.

| Module | Responsibility |
| --- | --- |
| `main` | Opens the database and the window and applies the saved theme. |
| `actions` | Actions and their key bindings. `secondary` maps to Cmd on macOS and Ctrl elsewhere. |
| `session` | The shared `Session` entity: saved clusters, the active one, its client, namespaces and the selected namespace. |
| `workspace` | The root view: sidebar, top bar, routing, dialogs and global shortcuts. |
| `pages::*` | One module per page, plus the cluster form. |
| `ui` | Small shared presentation helpers. |

## Threading model

GPUI renders on the main thread. Anything that may block runs on the
background executor with `cx.background_spawn`:

- keychain access (it can show an unlock prompt);
- every Kubernetes API call.

The result comes back to the entity with `cx.spawn` / `cx.spawn_in`. Two
mechanisms stop stale results from being shown:

1. **`Session::epoch`** increases whenever the cluster, its connection or the
   namespace changes. The session ignores a connection result if the epoch
   moved on.
2. **Per-page request counters** (`ResourcesPage::request`,
   `OverviewPage::request`). A response is applied only if no newer request
   was started.

## Data flow: opening a table

1. The user presses <kbd>Mod</kbd>+<kbd>2</kbd>. `Workspace` handles `GoPods`
   and calls `ResourcesPage::show(Pods)`.
2. `ResourcesPage::reload` takes the client and namespace from the `Session`
   and lists pods on a background thread.
3. `kube::table::rows` turns the JSON into `Row`s, still off the UI thread.
4. Back on the main thread, the rows go into `ResourceTable`, which filters
   and sorts them, and the `DataTable` re-renders.

## Data flow: applying an edit

1. `ManifestPage` parses the editor text (`manifest::parse`).
2. `manifest::prepare_update` compares it with the object as loaded. It
   refuses changes to `apiVersion`, `kind`, `metadata.name`,
   `metadata.namespace` or `metadata.uid`, and restores a removed
   `resourceVersion`.
3. `KubeClient::replace` sends a PUT. A `409 Conflict` becomes
   `KubeError::Conflict` with a message asking the user to reload.

## Testing

| Layer | How |
| --- | --- |
| Core logic | Unit tests next to the code (`cargo test -p kubiru-core`). |
| HTTP client | `crates/kubiru-core/tests/client.rs` runs `KubeClient` against a small local HTTP server and checks headers, paths, bodies, error mapping and that redirects are not followed. |
| UI | Headless GPUI tests (`#[gpui_kit::test]`) open the real `Workspace` in a window `Root` with an in-memory store and vault, then drive it with simulated keystrokes. |

## Adding a resource kind

1. Add a variant to `ResourceKind` and fill in every `match` in
   `kube/resource.rs`. The compiler lists them all.
2. Add its columns to `kube::table::columns` and a row builder to `row`.
3. Give it an icon in `workspace::kind_icon`.
4. Add a test in `kube/table.rs` with a realistic object.
