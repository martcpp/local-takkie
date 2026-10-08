# Changelog

All notable changes to local-takkie. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-10-08

### Added

- Add mdns

### Fixed

- Fix crate vulnerability
- Build libopus on Windows with CMake 4 and in debug builds (E1.8) (#128)

### Changed

- Refactor code
- Refactor code
- Fix formatting and clippy errors (#127)
- Move the code into takkie-engine and takkie-tui (E2.1b) (#131)

### Documentation

- Add project roadmap (#10)
- Rewrite the README (#129)
- Add ADRs for decisions D1 to D10 (#136)
- Add CONTRIBUTING.md, a PR template and issue templates (#137)
- Add a changelog generated with git-cliff (#149)

### Tests

- Prune tests that don't exercise project code (#123)

### Build and CI

- Replace the old workflows with ci.yml for all three OSes (#138)
- Run cargo-deny in CI (#139)
- Add a dist release workflow for the terminal app (#140)
- Add Dependabot for Cargo and GitHub Actions (E3.3) (#141)
- Bump actions/checkout from 6.1.0 to 7.0.1 (#144)

### Dependencies

- Bump the cargo-minor-and-patch group with 2 updates (#142)
- Bump mdns-sd from 0.17.1 to 0.20.0 (#145)
- Bump crossterm from 0.28.1 to 0.29.0 (#146)

### Maintenance

- Add Claude Code harness config (#122)
- Rename crate to local-takkie and binary to takkie (E1.2) (#124)
- Remove the 22 unused dependencies (E1.3) (#125)
- Delete obsolete files and tidy .gitignore (E1.4) (#126)
- Create the Cargo workspace and the three crates (#130)
- Share workspace metadata and lints across the crates (E2.2a) (#132)
- Add a release profile and rustfmt.toml (#133)
- Add cargo xtask for cross-platform dev commands (#134)
- Add cargo-deny for licences, advisories, bans and sources (#135)
- Add cargo xtask release to prepare a release in one command (E3.7) (#151)

### Other

- Initial commit
- Added udp featuree
- Breaking thing to files
- Did refactoring
- Starting audio processing
- Add tui
- Added tui
- Added set up for other machince
- Making the inde wokr for all othe rdevice
- Uodate
- Added libatk1.0-dev
- Added some tool for linux work
- Added linux dependices
- Feature/fix missing opus dependency (#1)
- Clippy fix (#2)
- Clippy fix (#5)
- Fix audio sample rate on linux and mac (#6)
- Fix audio sample rate on linux and mac (#7)
- Added to rad.rs
- Sample rate (#8)
- Ptt fix
- Ptt issue (#9)
- Ppt issue
- Made some fix
- Setting for prod
- Added cmd for build

