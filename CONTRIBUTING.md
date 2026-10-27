# Contributing to Kubiru

Thank you for helping to improve Kubiru! This guide explains how to propose
changes and what we look for in a pull request.

By taking part you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug** with the [bug report form](https://github.com/phoscoder/kubiru/issues/new?template=bug_report.yml).
- **Suggest a feature** with the [feature request form](https://github.com/phoscoder/kubiru/issues/new?template=feature_request.yml).
- **Improve the documentation.** Fixes to typos and unclear wording are always welcome.
- **Write code.** Look for issues labelled `good first issue` or `help wanted`, or open an issue to discuss your idea first.

Security issues are the exception: please follow [SECURITY.md](SECURITY.md)
instead of opening a public issue.

## Development setup

1. Install the latest stable Rust with [rustup](https://rustup.rs). The
   toolchain, `rustfmt` and `clippy` are selected automatically by
   `rust-toolchain.toml`.
2. On Linux, install the system packages listed in the
   [README](README.md#installation).
3. Clone and build:

   ```sh
   git clone https://github.com/phoscoder/kubiru.git
   cd kubiru
   cargo run
   ```

The first build compiles GPUI and takes a few minutes; later builds are
incremental.

### Trying changes without a real cluster

Any local cluster works, for example [kind](https://kind.sigs.k8s.io) or
[minikube](https://minikube.sigs.k8s.io). Create a token as described in
[Getting started](README.md#getting-started) and add the cluster in the app.
Never use production credentials while developing.

## Before you open a pull request

Run the same checks as CI:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Please also:

- **Add tests.** Logic belongs in `kubiru-core` with unit tests. UI behaviour
  can be covered with headless `#[gpui_kit::test]` tests; see
  `crates/kubiru/src/workspace.rs` for examples.
- **Keep the UI consistent.** Use GPUI Kit components and theme colours
  (never hard-coded colours), give every state (loading, empty, error) a
  clear message and a way forward, confirm destructive actions, and make
  new actions reachable from the keyboard.
- **Never weaken the security model** described in the
  [README](README.md#security). Tokens must not be logged, written to disk,
  or sent anywhere except the configured API server.
- **Update the documentation** (README, `docs/`, `CHANGELOG.md`) when behaviour changes.

## Coding guidelines

- Follow the existing style; `rustfmt` settings are in `rustfmt.toml`.
- `unsafe` code is forbidden. Avoid `unwrap()` and `expect()` outside tests;
  return errors and show them to the user.
- Do blocking work (network, keychain) on the background executor, never on
  the UI thread.
- Write comments for *why*, not *what*. Public items in `kubiru-core` should
  have doc comments.
- Keep dependencies few and well maintained. Explain any new dependency in
  your pull request.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<optional scope>): <summary in the imperative, lower case, no full stop>

<optional body explaining what and why, wrapped at 72 characters>
```

Common types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `ci`,
`chore`. Scopes in use: `core`, `app`.

Keep commits atomic: each one should build, pass the tests and do one thing.

## Pull request process

1. Fork the repository and create a branch from `main`
   (for example `feat/pod-logs` or `fix/namespace-reset`).
2. Make your change with tests and documentation.
3. Open a pull request and fill in the template. Link the issue it closes.
4. A maintainer will review it, usually within a week. Please respond to
   feedback by pushing new commits; we squash or rebase when merging.
5. Once CI passes and the change is approved, a maintainer merges it.

## Licensing

Kubiru is licensed under the [MIT License](LICENSE). By submitting a
contribution, you agree that it is provided under the same licence.
