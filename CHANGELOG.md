# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-27

The first release of the native rewrite. Kubiru is now a desktop
application written in Rust with GPUI Kit, replacing the earlier Next.js
web application.

### Added

- Save multiple clusters with an API server address, bearer token, optional
  CA certificate and description; switch between them from the sidebar.
- Test a connection before saving a cluster.
- Overview with node and deployment health, Kubernetes version, pod phases
  and the failing pods with the most restarts.
- Tables for 13 resource kinds with kubectl-style columns and statuses,
  natural sorting, filtering and keyboard navigation.
- Manifest viewer and editor in YAML or JSON with syntax highlighting.
- Applying edits and deleting objects, with confirmation and conflict detection.
- Searchable namespace switcher remembered per cluster.
- Light, dark and system appearance.
- Keyboard shortcuts for all main views and actions.

### Security

- Tokens are stored in the operating system keychain instead of the local
  database, and are never logged.
- TLS certificates are verified by default, with optional custom CAs; the
  previous version disabled verification for every request.
- Redirects are refused, plain HTTP is limited to loopback, object names
  are validated before use in URLs, and requests are time- and size-limited.
- Edits cannot change an object's identity or overwrite concurrent changes.

### Removed

- The Next.js web application, NextAuth sign-in and template pages.

[Unreleased]: https://github.com/phoscoder/kubiru/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/phoscoder/kubiru/releases/tag/v0.1.0
