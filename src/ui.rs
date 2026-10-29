//! All ratatui rendering.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, DetailLine, Focus, InputMode, Mode};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const ERR: Color = Color::Red;
const OK: Color = Color::Green;
const SIGNAL: Color = Color::Yellow;

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    match app.mode {
        Mode::Explore => draw_explore(f, app, root[0]),
        Mode::Monitor => draw_monitor(f, app, root[0]),
    }
    draw_statusbar(f, app, root[1]);

    if let Some((text, is_err)) = &app.last_result {
        draw_popup(
            f,
            if *is_err { " error " } else { " result " },
            text,
            if *is_err { ERR } else { OK },
        );
    }
    if app.show_help {
        draw_help(f);
    }
}

// ---- explore view ------------------------------------------------------------

fn draw_explore(f: &mut Frame, app: &App, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(30),
            Constraint::Percentage(44),
        ])
        .split(area);

    draw_services(f, app, panes[0]);
    draw_objects(f, app, panes[1]);
    draw_details(f, app, panes[2]);
}

fn pane_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::default().bg(ACCENT).fg(Color::Black)
    } else {
        Style::default().bg(Color::Rgb(40, 40, 40))
    }
}

fn draw_services(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Services;
    let services = app.filtered_services();
    let items: Vec<ListItem> = services
        .iter()
        .map(|s| {
            let name_style = if s.well_known {
                Style::default()
            } else {
                Style::default().fg(DIM)
            };
            let mut spans = vec![Span::styled(s.name.clone(), name_style)];
            if let Some(owner) = &s.owner {
                if s.well_known {
                    spans.push(Span::styled(format!("  {owner}"), Style::default().fg(DIM)));
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = if app.service_filter.is_empty() {
        format!("services ({})", services.len())
    } else {
        format!("services ({})  /{}", services.len(), app.service_filter)
    };
    let list = List::new(items)
        .block(pane_block(&title, focused))
        .highlight_style(highlight(focused))
        .highlight_symbol("» ");
    let mut state = ListState::default().with_selected(if services.is_empty() {
        None
    } else {
        Some(app.sel_service)
    });
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_objects(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Objects;
    let flat = app.flatten_objects();
    let block = pane_block("objects", focused);

    if app.selected_service().is_none() {
        f.render_widget(
            Paragraph::new("no service selected").block(block).fg(DIM),
            area,
        );
        return;
    }
    if flat.is_empty() {
        f.render_widget(
            Paragraph::new("press Enter to load object tree")
                .block(block)
                .fg(DIM),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = flat
        .iter()
        .map(|fo| {
            let indent = "  ".repeat(fo.depth);
            let marker = if fo.loading {
                "…"
            } else if fo.error {
                "✗"
            } else if fo.has_children || !fo.loaded {
                if app.expanded.contains(&fo.path) {
                    "▾"
                } else {
                    "▸"
                }
            } else {
                "·"
            };
            let style = if fo.error {
                Style::default().fg(ERR)
            } else if fo.loading {
                Style::default().fg(DIM)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{indent}{marker} {}", fo.name),
                style,
            )))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused))
        .highlight_symbol("» ");
    let mut state = ListState::default().with_selected(Some(app.sel_object));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_details(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Details;
    let block = pane_block("details", focused);

    let Some(node) = app.selected_node() else {
        f.render_widget(
            Paragraph::new("select an object to inspect")
                .block(block)
                .fg(DIM),
            area,
        );
        return;
    };
    if node.loading {
        f.render_widget(Paragraph::new("introspecting…").block(block).fg(DIM), area);
        return;
    }
    if let Some(err) = &node.error {
        f.render_widget(
            Paragraph::new(format!("introspection failed:\n{err}"))
                .block(block)
                .fg(ERR)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    if node.interfaces.is_empty() {
        f.render_widget(
            Paragraph::new("no interfaces on this object")
                .block(block)
                .fg(DIM),
            area,
        );
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    for line in &app.detail_lines {
        match *line {
            DetailLine::Header { iface } => {
                let i = &node.interfaces[iface];
                let toggle = if i.collapsed { "[+]" } else { "[-]" };
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("{toggle} {}", i.name),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))));
            }
            DetailLine::Method { iface, method } => {
                let m = &node.interfaces[iface].methods[method];
                let ins: Vec<String> = m.in_args.iter().map(fmt_arg).collect();
                let outs: Vec<String> = m.out_args.iter().map(fmt_arg).collect();
                let sig = if outs.is_empty() {
                    format!("({})", ins.join(", "))
                } else {
                    format!("({}) → ({})", ins.join(", "), outs.join(", "))
                };
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("    ƒ ", Style::default().fg(OK)),
                    Span::raw(&m.name),
                    Span::styled(sig, Style::default().fg(DIM)),
                ])));
            }
            DetailLine::Signal { iface, signal } => {
                let s = &node.interfaces[iface].signals[signal];
                let args: Vec<String> = s.args.iter().map(fmt_arg).collect();
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("    ~ ", Style::default().fg(SIGNAL)),
                    Span::raw(&s.name),
                    Span::styled(format!("({})", args.join(", ")), Style::default().fg(DIM)),
                ])));
            }
            DetailLine::Property { iface, prop } => {
                let i = &node.interfaces[iface];
                let p = &i.properties[prop];
                let value = app
                    .selected_node()
                    .and_then(|n| n.prop_values.get(&i.name))
                    .and_then(|vals| vals.iter().find(|(k, _)| k == &p.name))
                    .map(|(_, v)| format!(" = {v}"))
                    .unwrap_or_default();
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("    = ", Style::default().fg(Color::Magenta)),
                    Span::raw(&p.name),
                    Span::styled(
                        format!(": {} ({}){value}", p.ty, p.access),
                        Style::default().fg(DIM),
                    ),
                ])));
            }
        }
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused))
        .highlight_symbol("» ");
    let mut state = ListState::default().with_selected(if app.detail_lines.is_empty() {
        None
    } else {
        Some(app.sel_detail)
    });
    f.render_stateful_widget(list, area, &mut state);
}

// ---- monitor view --------------------------------------------------------------

fn draw_monitor(f: &mut Frame, app: &App, area: Rect) {
    let signals = app.filtered_signals();
    let title = if app.monitor_filter.is_empty() {
        format!(" signal monitor, times UTC ({}) ", signals.len())
    } else {
        format!(
            " signal monitor, times UTC ({})  /{} ",
            signals.len(),
            app.monitor_filter
        )
    };
    let items: Vec<ListItem> = signals
        .iter()
        .map(|e| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", e.time), Style::default().fg(DIM)),
                Span::styled(&e.interface, Style::default().fg(SIGNAL)),
                Span::styled(format!(".{}", e.member), Style::default().fg(ACCENT)),
                Span::styled(format!("  {}", e.path), Style::default().fg(DIM)),
                Span::raw(format!("  {}", truncate(&e.body, 120))),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(pane_block(&title, true))
        .highlight_style(highlight(true))
        .highlight_symbol("» ");
    let selected = if signals.is_empty() {
        None
    } else if app.monitor_follow {
        Some(signals.len() - 1)
    } else {
        Some(app.sel_monitor.min(signals.len() - 1))
    };
    let mut state = ListState::default().with_selected(selected);
    f.render_stateful_widget(list, area, &mut state);
}

// ---- status bar / input ---------------------------------------------------------

fn draw_statusbar(f: &mut Frame, app: &App, area: Rect) {
    let line = match &app.input_mode {
        InputMode::FilterServices => Line::from(vec![
            Span::styled(" filter services: ", Style::default().fg(ACCENT)),
            Span::raw(&app.input),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]),
        InputMode::MonitorFilter => Line::from(vec![
            Span::styled(" filter signals: ", Style::default().fg(ACCENT)),
            Span::raw(&app.input),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]),
        InputMode::CallArgs {
            iface,
            method,
            hint,
            ..
        } => Line::from(vec![
            Span::styled(
                format!(" {iface}.{method}  "),
                Style::default().fg(OK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.input.is_empty() {
                    format!("{hint}  ")
                } else {
                    String::new()
                },
                Style::default().fg(DIM),
            ),
            Span::raw(&app.input),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(
                if app.status_error {
                    format!("   {}", app.status)
                } else {
                    String::new()
                },
                Style::default().fg(ERR),
            ),
        ]),
        InputMode::Normal => {
            let bus = if app.client.system {
                "system"
            } else {
                "session"
            };
            let style = if app.status_error {
                Style::default().fg(ERR)
            } else {
                Style::default().fg(DIM)
            };
            Line::from(vec![
                Span::styled(
                    format!(" {bus} bus "),
                    Style::default().fg(Color::Black).bg(ACCENT),
                ),
                Span::raw("  "),
                Span::styled(&app.status, style),
                Span::styled(
                    match app.mode {
                        Mode::Explore => "   ?:help  q:quit  m:monitor  Tab:pane  /:filter",
                        Mode::Monitor => "   ?:help  Esc:back  /:filter  Enter:details  x:clear",
                    },
                    Style::default().fg(DIM),
                ),
            ])
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

// ---- popups -----------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn draw_popup(f: &mut Frame, title: &str, text: &str, color: Color) {
    let area = centered_rect(70, 50, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text.to_string())
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(color)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_help(f: &mut Frame) {
    let help = "\
global
  q, Ctrl-C   quit                    ?        this help
  m           toggle signal monitor   Tab      switch pane

explore view
  j/k, ↑/↓    move selection          Enter    select / expand / collapse
  ←           collapse / go back      r        refresh service list
  /           filter services (Esc clears)
  g           fetch property values of the interface under the cursor
  Enter on a method line: call it; type its arguments in order, separated
  by spaces (\"quote\" values containing spaces). Esc cancels.

monitor view
  m / Esc     back to explorer        /        filter signals
  x           clear buffer            G        jump to newest (follow)
  j/k         scroll (stops follow)   Enter    full signal details

argument values
  s o g       text    b  true/false   y n q i u x t d  numbers
  as ao au …  comma separated, e.g. a,b,c
  a{..} / any array   '' for empty
  v           sig:value, e.g. s:hi or u:3
  structs and non-empty dicts are not supported";
    let area = centered_rect(72, 66, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(help)
            .block(
                Block::default()
                    .title(" busk help ")
                    .title_alignment(Alignment::Center)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn fmt_arg(a: &crate::model::ArgInfo) -> String {
    if a.name.is_empty() {
        a.ty.clone()
    } else {
        format!("{} {}", a.ty, a.name)
    }
}
