# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-10-28

### Changed
- README: replaced the text screenshots with GIF demos of exploring, calling methods and
  watching signals.
- Demo assets are excluded from the published crate.

## [0.1.0] - 2026-10-28

Initial release.

### Added
- Three-pane explorer: bus services, lazily introspected object tree, and per-object
  interfaces with their methods, signals and properties.
- Method calls with arguments parsed against the method's declared signature (quoting,
  arrays of basic types, empty arrays/dicts, `v` as `sig:value`).
- Property values via `GetAll`, with a popup for the full value.
- Live signal monitor with filter, follow mode and a full-detail view per signal.
- Session (default) and system (`--system`) bus support; `--list` prints bus names.
- Incremental service filter, bracketed paste in prompts, in-app help (`?`).

[0.1.1]: https://github.com/LoganBaird-Levenick/busk/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/LoganBaird-Levenick/busk/releases/tag/v0.1.0
