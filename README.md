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

## Screenshots

Exploring `org.freedesktop.Notifications`:

```text
┌ services (1)  /org.freedeskto┐┌ objects ──────────────────────────┐┌ details ────────────────────────────────────────────┐
│» org.freedesktop.Notification││  ▾ /                              ││  [-] org.freedesktop.DBus.Properties                │
│                              ││    ▾ org                          ││      ƒ Get(s interface_name, s property_name) → (v v│
│                              ││      ▾ freedesktop                ││      ƒ GetAll(s interface_name) → (a{sv} properties)│
│                              ││»       · Notifications            ││      ƒ Set(s interface_name, s property_name, v valu│
│                              ││                                   ││      ~ PropertiesChanged(s interface_name, a{sv} cha│
│                              ││                                   ││  [-] org.freedesktop.DBus.Introspectable            │
│                              ││                                   ││      ƒ Introspect() → (s xml_data)                  │
│                              ││                                   ││» [-] org.freedesktop.DBus.Peer                      │
│                              ││                                   ││      ƒ Ping()                                       │
│                              ││                                   ││      ƒ GetMachineId() → (s machine_uuid)            │
│                              ││                                   ││  [-] org.freedesktop.Notifications                  │
│                              ││                                   ││      ƒ Notify(s arg_0, u arg_1, s arg_2, s arg_3, s │
│                              ││                                   ││      ƒ CloseNotification(u arg_0)                   │
│                              ││                                   ││      ƒ GetCapabilities() → (as arg_0)               │
│                              ││                                   ││      ƒ GetServerInformation() → (s arg_0, s arg_1, s│
│                              ││                                   ││      ~ NotificationClosed(u arg_0, u arg_1)         │
│                              ││                                   ││      ~ ActionInvoked(u arg_0, s arg_1)              │
│                              ││                                   ││      ~ ActivationToken(u arg_0, s arg_1)            │
│                              ││                                   ││                                                     │
└──────────────────────────────┘└───────────────────────────────────┘└─────────────────────────────────────────────────────┘
 session bus   introspected /org/freedesktop/Notifications   ?:help  q:quit  m:monitor  Tab:pane  /:filter
```

Live signal monitor, filtered to `org.example`:

```text
┌  signal monitor, times UTC (4)  /org.example  ───────────────────────────────────────────────────────────────────────────┐
│  03:02:44 org.example.Player.TrackChanged  /demo/Player  "hello world"  42  ["a", "b"]                                   │
│  03:02:44 org.example.Player.TrackChanged  /demo/Player  true                                                            │
│  03:02:44 org.example.Player.Stopped  /demo/Player  ()                                                                   │
│» 03:02:44 org.example.Mixer.VolumeChanged  /demo/Mixer  0.75                                                             │
│                                                                                                                          │
│                                                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 session bus   signal monitor started   ?:help  Esc:back  /:filter  Enter:details  x:clear
```

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
