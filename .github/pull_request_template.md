Closes #

## What

<!-- What changes and why. One ticket per PR. -->

## Self-review

- [ ] `cargo xtask ci` passes
- [ ] Behaviour changes have tests; bug fixes have a test that fails without the fix
- [ ] Docs updated where needed (rustdoc, README, CLAUDE.md, ROADMAP, ADR)
- [ ] No `unwrap`/`expect` in non-test code, and no new `#[allow]` or `#[ignore]`
- [ ] Audio callbacks still don't lock, allocate, log or block
