## What does this change?

<!-- A short summary. Link the issue it closes, e.g. "Closes #12". -->

## How was it tested?

<!-- Tests added or updated, and anything you checked by hand. -->

## Screenshots

<!-- For visible changes, before and after, in light and dark mode if relevant. -->

## Checklist

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] New behaviour is covered by tests
- [ ] Loading, empty and error states are handled, and destructive actions ask for confirmation
- [ ] No token or other secret can be logged, stored on disk or sent elsewhere than the configured cluster
- [ ] Documentation and `CHANGELOG.md` are updated
