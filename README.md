<div align="center">

# busk

**A keyboard-driven D-Bus explorer and debugger for your terminal.**

Browse objects · Call methods · Watch signals live

[![Crates.io](https://img.shields.io/crates/v/busk.svg?logo=rust&color=orange)](https://crates.io/crates/busk)
[![Downloads](https://img.shields.io/crates/d/busk.svg?color=blue)](https://crates.io/crates/busk)
[![CI](https://img.shields.io/github/actions/workflow/status/LoganBaird-Levenick/busk/ci.yml?branch=master&logo=github&label=CI)](https://github.com/LoganBaird-Levenick/busk/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.88+-orange.svg?logo=rust)](https://blog.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-linux-lightgrey.svg?logo=linux&logoColor=white)](#install)
[![Built with Ratatui](https://img.shields.io/badge/built%20with-ratatui-blueviolet.svg)](https://ratatui.rs)

</div>

---

`busk` is a terminal UI for poking at D-Bus. Think `d-feet` or `bustle`, but in your
terminal, driven entirely from the keyboard, and pure Rust (no `libdbus` required).

## Features

- **Browse** every name on the session or system bus, with an incremental filter.
- **Explore** each service's object tree. Nodes are introspected lazily as you move.
- **Inspect** interfaces: methods, signals and properties with their D-Bus signatures.
- **Call methods** from the details pane. Arguments are typed against the method's own
  signature, so you just type values. Results and errors come back in a popup.
- **Read properties** with `GetAll`; open any value in full.
- **Watch signals live** across the whole bus, with a filter, follow mode and a per-signal
  detail view.
- Session bus by default, `--system` for the system bus, `--list` for scripting.

## Demo

### Explore services, objects and properties

![Exploring a service: filter, walk the object tree, read property values](https://raw.githubusercontent.com/LoganBaird-Levenick/busk/master/assets/explore.gif)

### Call methods with typed arguments

![Calling Echo and Add, including an argument error caught before sending](https://raw.githubusercontent.com/LoganBaird-Levenick/busk/master/assets/call.gif)

### Watch signals live

![Live signal monitor with filter and per-signal detail view](https://raw.githubusercontent.com/LoganBaird-Levenick/busk/master/assets/monitor.gif)

<sub>Recorded with [VHS](https://github.com/charmbracelet/vhs) against a private bus that only
hosts a small demo service, so no real desktop services appear.</sub>

## Install

### From crates.io

```sh
cargo install busk
```

Requires Rust 1.88 or newer.

### From source

```sh
git clone https://github.com/LoganBaird-Levenick/busk
cd busk
cargo install --path .
```

### Prebuilt binaries

Linux binaries are attached to each [GitHub release](https://github.com/LoganBaird-Levenick/busk/releases).

## Usage

```sh
busk              # explore the session bus
busk --system     # explore the system bus
busk --list       # print bus names and their owners, then exit
```

### Keys

| Key | Action |
|---|---|
| `j` / `k`, `↑` / `↓` | Move selection |
| `Enter` | Load a service, expand a node, call a method, or show a property's full value |
| `→` | Expand, then step into the details pane (never collapses) |
| `←` | Collapse, or go back one pane |
| `Tab` | Switch pane |
| `/` | Filter services (or signals in the monitor); `Esc` clears the filter |
| `g` | Fetch property values for the interface under the cursor |
| `r` | Refresh the service list |
| `m` | Toggle the live signal monitor |
| `?` | Help |
| `q`, `Ctrl-C` | Quit |

In the monitor: `Enter` opens full signal details, `x` clears the buffer, `G` jumps to the
newest signal and resumes following, `j` / `k` scroll.

### Method arguments

Press `Enter` on a method and type its arguments in order, separated by spaces. Quote
values that contain spaces.

| Type | Input |
|---|---|
| `s` `o` `g` | text |
| `b` | `true` / `false` |
| `y` `n` `q` `i` `u` `x` `t` `d` | numbers |
| arrays of the above (`as`, `au`, `ao`, ...) | comma separated: `a,b,c` |
| any array or dict (`a{sv}`, ...) | `''` for empty |
| `v` | `sig:value`, e.g. `u:3` |

Example, sending a desktop notification with `org.freedesktop.Notifications.Notify`:

```text
busk 0 "" "Hello" "from busk" '' '' -1
```

## Limitations

- Struct arguments and non-empty dict/nested-array arguments can't be entered yet.
- There is no dedicated property editor; to write a property call
  `org.freedesktop.DBus.Properties.Set` (`s s v`) like any other method.
- Monitor timestamps are UTC.
- Calling methods on a live bus has real side effects. Mind what you call, especially on the
  system bus.

## Contributing

Issues and pull requests are welcome. Before sending a PR:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

See the [changelog](CHANGELOG.md) for release history.

## License

[MIT](LICENSE) © Logan Baird-Levenick
