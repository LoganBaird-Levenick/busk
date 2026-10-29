//! Application state, event loop, and input handling.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use crate::bus::{parse_args, BusClient};
use crate::model::*;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Explore,
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Services,
    Objects,
    Details,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Services => Focus::Objects,
            Focus::Objects => Focus::Details,
            Focus::Details => Focus::Services,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// `/` in the services pane.
    FilterServices,
    /// `/` in the monitor view.
    MonitorFilter,
    /// Entering arguments for a method call.
    CallArgs {
        service: String,
        path: String,
        iface: String,
        method: String,
        hint: String,
        /// Declared D-Bus type of each input argument.
        sigs: Vec<String>,
    },
}

/// A selectable line in the details pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailLine {
    Header { iface: usize },
    Method { iface: usize, method: usize },
    Signal { iface: usize, signal: usize },
    Property { iface: usize, prop: usize },
}

/// Flattened row of the object tree for rendering/selection.
#[derive(Debug, Clone)]
pub struct FlatObject {
    pub depth: usize,
    pub name: String,
    pub path: String,
    pub loaded: bool,
    pub loading: bool,
    pub error: bool,
    pub has_children: bool,
}

/// Events sent back to the UI from background D-Bus tasks.
pub enum BusEvent {
    Services(Result<Vec<ServiceEntry>, String>),
    Introspected {
        service: String,
        path: String,
        result: Result<(Vec<InterfaceInfo>, Vec<String>), String>,
    },
    Props {
        service: String,
        path: String,
        iface: String,
        result: Result<Vec<(String, String)>, String>,
    },
    CallDone {
        what: String,
        result: Result<String, String>,
    },
    MonitorStarted(Result<(), String>),
    Signal(SignalEvent),
}

const MONITOR_CAP: usize = 2000;

pub struct App {
    pub client: BusClient,
    pub mode: Mode,
    pub focus: Focus,
    pub input_mode: InputMode,
    pub input: String,
    pub running: bool,
    pub show_help: bool,

    // explore view
    pub services: Vec<ServiceEntry>,
    pub service_filter: String,
    pub sel_service: usize,
    pub objects: HashMap<String, ObjectNode>,
    pub expanded: HashSet<String>,
    pub sel_object: usize,
    pub detail_lines: Vec<DetailLine>,
    pub sel_detail: usize,

    // monitor view
    pub monitor: VecDeque<SignalEvent>,
    pub monitor_filter: String,
    pub monitor_started: bool,
    pub monitor_follow: bool,
    pub sel_monitor: usize,

    // feedback
    pub status: String,
    pub status_error: bool,
    pub last_result: Option<(String, bool)>, // (text, is_error)

    tx: mpsc::UnboundedSender<BusEvent>,
    rx: mpsc::UnboundedReceiver<BusEvent>,
}

impl App {
    fn new(client: BusClient) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            client,
            mode: Mode::Explore,
            focus: Focus::Services,
            input_mode: InputMode::Normal,
            input: String::new(),
            running: true,
            show_help: false,
            services: Vec::new(),
            service_filter: String::new(),
            sel_service: 0,
            objects: HashMap::new(),
            expanded: HashSet::new(),
            sel_object: 0,
            detail_lines: Vec::new(),
            sel_detail: 0,
            monitor: VecDeque::new(),
            monitor_filter: String::new(),
            monitor_started: false,
            monitor_follow: true,
            sel_monitor: 0,
            status: "loading services…".into(),
            status_error: false,
            last_result: None,
            tx,
            rx,
        }
    }

    // ---- selection helpers -------------------------------------------------

    pub fn filtered_services(&self) -> Vec<&ServiceEntry> {
        let f = self.service_filter.to_lowercase();
        self.services
            .iter()
            .filter(|s| f.is_empty() || s.name.to_lowercase().contains(&f))
            .collect()
    }

    pub fn selected_service(&self) -> Option<&ServiceEntry> {
        self.filtered_services().get(self.sel_service).copied()
    }

    pub fn flatten_objects(&self) -> Vec<FlatObject> {
        let mut out = Vec::new();
        let Some(service) = self.selected_service() else {
            return out;
        };
        let Some(root) = self.objects.get(&service.name) else {
            return out;
        };
        flatten_node(root, 0, &self.expanded, &mut out);
        out
    }

    pub fn selected_object_path(&self) -> Option<String> {
        self.flatten_objects()
            .get(self.sel_object)
            .map(|f| f.path.clone())
    }

    pub fn selected_node(&self) -> Option<&ObjectNode> {
        let service = self.selected_service()?;
        let root = self.objects.get(&service.name)?;
        let path = self.selected_object_path()?;
        find_node(root, &path)
    }

    fn selected_node_mut(&mut self) -> Option<&mut ObjectNode> {
        let service = self.selected_service()?.name.clone();
        let path = self.selected_object_path()?;
        let root = self.objects.get_mut(&service)?;
        find_node_mut(root, &path)
    }

    pub fn filtered_signals(&self) -> Vec<&SignalEvent> {
        let f = self.monitor_filter.to_lowercase();
        self.monitor
            .iter()
            .filter(|e| {
                f.is_empty()
                    || e.interface.to_lowercase().contains(&f)
                    || e.member.to_lowercase().contains(&f)
                    || e.path.to_lowercase().contains(&f)
                    || e.sender.to_lowercase().contains(&f)
            })
            .collect()
    }

    // ---- status ------------------------------------------------------------

    fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = msg.into();
        self.status_error = is_error;
    }

    // ---- background tasks ----------------------------------------------------

    fn spawn_services(&self) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = client.list_services().await.map_err(|e| e.to_string());
            let _ = tx.send(BusEvent::Services(result));
        });
    }

    fn spawn_introspect(&self, service: String, path: String) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = client
                .introspect(&service, &path)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BusEvent::Introspected {
                service,
                path,
                result,
            });
        });
    }

    fn spawn_props(&self, service: String, path: String, iface: String) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = client
                .get_all_properties(&service, &path, &iface)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BusEvent::Props {
                service,
                path,
                iface,
                result,
            });
        });
    }

    fn spawn_call(
        &self,
        service: String,
        path: String,
        iface: String,
        method: String,
        args: Vec<zbus::zvariant::Value<'static>>,
    ) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        let what = format!("{iface}.{method}");
        tokio::spawn(async move {
            let result = client
                .call_method(&service, &path, &iface, &method, args)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BusEvent::CallDone { what, result });
        });
    }

    // ---- state updates -------------------------------------------------------

    fn ensure_service_root(&mut self) {
        let Some(service) = self.selected_service().map(|s| s.name.clone()) else {
            return;
        };
        if !self.objects.contains_key(&service) {
            let mut root = ObjectNode::new("/".into());
            root.loading = true;
            self.objects.insert(service.clone(), root);
            self.expanded.insert("/".into());
            self.spawn_introspect(service, "/".into());
        }
    }

    fn rebuild_detail_lines(&mut self) {
        let mut lines = Vec::new();
        if let Some(node) = self.selected_node() {
            for (i, iface) in node.interfaces.iter().enumerate() {
                lines.push(DetailLine::Header { iface: i });
                if iface.collapsed {
                    continue;
                }
                for (m, _) in iface.methods.iter().enumerate() {
                    lines.push(DetailLine::Method {
                        iface: i,
                        method: m,
                    });
                }
                for (s, _) in iface.signals.iter().enumerate() {
                    lines.push(DetailLine::Signal {
                        iface: i,
                        signal: s,
                    });
                }
                for (p, _) in iface.properties.iter().enumerate() {
                    lines.push(DetailLine::Property { iface: i, prop: p });
                }
            }
        }
        self.detail_lines = lines;
        if self.sel_detail >= self.detail_lines.len() {
            self.sel_detail = self.detail_lines.len().saturating_sub(1);
        }
    }

    fn handle_bus_event(&mut self, ev: BusEvent) {
        match ev {
            BusEvent::Services(Ok(list)) => {
                self.set_status(format!("{} names on the bus", list.len()), false);
                self.services = list;
                self.clamp_service_selection();
                self.rebuild_detail_lines();
            }
            BusEvent::Services(Err(e)) => {
                self.set_status(format!("listing names failed: {e}"), true);
            }
            BusEvent::Introspected {
                service,
                path,
                result,
            } => {
                // Scope the tree borrow so we can update status afterwards.
                let status: Option<(String, bool)> = {
                    let Some(root) = self.objects.get_mut(&service) else {
                        return;
                    };
                    let Some(node) = find_node_mut(root, &path) else {
                        return;
                    };
                    match result {
                        Ok((interfaces, children)) => {
                            node.interfaces = interfaces;
                            node.children = children
                                .into_iter()
                                .map(|c| ObjectNode::new(join_path(&path, &c)))
                                .collect();
                            node.loaded = true;
                            node.loading = false;
                            node.error = None;
                            Some((format!("introspected {path}"), false))
                        }
                        Err(e) => {
                            node.loading = false;
                            node.error = Some(e.clone());
                            Some((format!("{path}: {e}"), true))
                        }
                    }
                };
                if let Some((msg, is_err)) = status {
                    self.set_status(msg, is_err);
                }
                self.rebuild_detail_lines();
            }
            BusEvent::Props {
                service,
                path,
                iface,
                result,
            } => match result {
                Ok(vals) => {
                    if let Some(root) = self.objects.get_mut(&service) {
                        if let Some(node) = find_node_mut(root, &path) {
                            node.prop_values.insert(iface.clone(), vals);
                        }
                    }
                    self.set_status(format!("fetched properties of {iface}"), false);
                    self.rebuild_detail_lines();
                }
                Err(e) => self.set_status(format!("GetAll {iface} failed: {e}"), true),
            },
            BusEvent::CallDone { what, result } => match result {
                Ok(out) => {
                    self.last_result = Some((format!("{what} returned:\n{out}"), false));
                    self.set_status(format!("{what} ok"), false);
                }
                Err(e) => {
                    self.last_result = Some((format!("{what} failed:\n{e}"), true));
                    self.set_status(format!("{what} failed"), true);
                }
            },
            BusEvent::MonitorStarted(Ok(())) => {
                self.set_status("signal monitor started", false);
            }
            BusEvent::MonitorStarted(Err(e)) => {
                self.set_status(format!("monitor failed: {e}"), true);
            }
            BusEvent::Signal(ev) => {
                if self.monitor.len() >= MONITOR_CAP {
                    self.monitor.pop_front();
                    if self.sel_monitor > 0 {
                        self.sel_monitor -= 1;
                    }
                }
                self.monitor.push_back(ev);
            }
        }
    }

    fn clamp_service_selection(&mut self) {
        let len = self.filtered_services().len();
        if len == 0 {
            self.sel_service = 0;
        } else if self.sel_service >= len {
            self.sel_service = len - 1;
        }
        let objects = self.flatten_objects().len();
        self.sel_object = self.sel_object.min(objects.saturating_sub(1));
        self.rebuild_detail_lines();
    }

    // ---- input ---------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Popups swallow keys first.
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.last_result.is_some() {
            self.last_result = None;
            return;
        }

        // Text entry modes.
        if self.input_mode != InputMode::Normal {
            self.on_input_key(key);
            return;
        }

        // Global keys.
        match (key.modifiers, key.code) {
            (m, KeyCode::Char('c')) if m.contains(KeyModifiers::CONTROL) => {
                self.running = false;
                return;
            }
            (_, KeyCode::Char('q')) => {
                self.running = false;
                return;
            }
            (_, KeyCode::Char('?')) => {
                self.show_help = true;
                return;
            }
            (_, KeyCode::Char('m')) => {
                self.toggle_monitor();
                return;
            }
            _ => {}
        }

        match self.mode {
            Mode::Explore => self.on_explore_key(key),
            Mode::Monitor => self.on_monitor_key(key),
        }
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => self.running = false,
                KeyCode::Char('u') => {
                    self.input.clear();
                    self.live_filter();
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc => {
                match self.input_mode {
                    InputMode::CallArgs { .. } => self.set_status("call cancelled", false),
                    // Esc in a filter prompt clears the filter.
                    _ => {
                        self.input.clear();
                        self.live_filter();
                    }
                }
                self.input_mode = InputMode::Normal;
                self.input.clear();
            }
            KeyCode::Enter => {
                if let InputMode::CallArgs {
                    service,
                    path,
                    iface,
                    method,
                    sigs,
                    ..
                } = &self.input_mode
                {
                    let sig_refs: Vec<&str> = sigs.iter().map(String::as_str).collect();
                    match parse_args(&sig_refs, &self.input) {
                        Ok(parsed) => {
                            let (service, path, iface, method) =
                                (service.clone(), path.clone(), iface.clone(), method.clone());
                            self.input_mode = InputMode::Normal;
                            self.input.clear();
                            self.set_status(format!("calling {iface}.{method}…"), false);
                            self.spawn_call(service, path, iface, method, parsed);
                        }
                        // Stay in the prompt so the line can be fixed.
                        Err(e) => self.set_status(format!("bad arguments: {e:#}"), true),
                    }
                    return;
                }
                self.input_mode = InputMode::Normal;
                self.input.clear();
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.live_filter();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.live_filter();
            }
            _ => {}
        }
    }

    /// Bracketed paste: only meaningful while a text prompt is open.
    fn on_paste(&mut self, text: &str) {
        if self.input_mode == InputMode::Normal {
            return;
        }
        self.input.extend(text.chars().filter(|c| !c.is_control()));
        self.live_filter();
    }

    fn live_filter(&mut self) {
        match self.input_mode {
            InputMode::FilterServices => {
                self.service_filter = self.input.clone();
                self.clamp_service_selection();
            }
            InputMode::MonitorFilter => {
                self.monitor_filter = self.input.clone();
            }
            _ => {}
        }
    }

    fn on_explore_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = self.focus.next();
            }
            KeyCode::Char('r') => {
                self.set_status("refreshing…", false);
                self.spawn_services();
            }
            KeyCode::Char('/') => {
                self.input = self.service_filter.clone();
                self.input_mode = InputMode::FilterServices;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(20),
            KeyCode::PageUp => self.move_selection(-20),
            KeyCode::Enter => self.activate(),
            KeyCode::Right => self.descend(),
            KeyCode::Left => self.collapse_or_back(),
            KeyCode::Char('g') => self.fetch_properties(),
            _ => {}
        }
    }

    fn on_monitor_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Explore;
            }
            KeyCode::Char('/') => {
                self.input = self.monitor_filter.clone();
                self.input_mode = InputMode::MonitorFilter;
            }
            KeyCode::Char('x') => {
                self.monitor.clear();
                self.sel_monitor = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.filtered_signals().len();
                if len > 0 {
                    if self.monitor_follow {
                        self.sel_monitor = len - 1;
                    }
                    self.monitor_follow = false;
                    self.sel_monitor = (self.sel_monitor + 1).min(len - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.filtered_signals().len();
                if len > 0 && self.monitor_follow {
                    self.sel_monitor = len - 1;
                }
                self.monitor_follow = false;
                self.sel_monitor = self.sel_monitor.saturating_sub(1);
            }
            KeyCode::Enter => {
                let len = self.filtered_signals().len();
                let idx = if self.monitor_follow {
                    len.checked_sub(1)
                } else {
                    Some(self.sel_monitor.min(len.saturating_sub(1)))
                };
                if let Some(e) = idx.and_then(|i| self.filtered_signals().get(i).copied()) {
                    let text = format!(
                        "{} (UTC)\nsender:      {}\ndestination: {}\npath:        {}\ninterface:   {}\nmember:      {}\n\n{}",
                        e.time,
                        e.sender,
                        if e.destination.is_empty() { "(broadcast)" } else { &e.destination },
                        e.path,
                        e.interface,
                        e.member,
                        e.body
                    );
                    self.last_result = Some((text, false));
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.monitor_follow = true;
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        match self.focus {
            Focus::Services => {
                let len = self.filtered_services().len();
                if len == 0 {
                    return;
                }
                let new = self.sel_service as i32 + delta;
                self.sel_service = new.clamp(0, len as i32 - 1) as usize;
                // Reset object selection if we moved to a different service.
                self.sel_object = 0;
                self.rebuild_detail_lines();
            }
            Focus::Objects => {
                let len = self.flatten_objects().len();
                if len == 0 {
                    return;
                }
                let new = self.sel_object as i32 + delta;
                self.sel_object = new.clamp(0, len as i32 - 1) as usize;
                self.sel_detail = 0;
                self.rebuild_detail_lines();
                self.load_selected_object();
            }
            Focus::Details => {
                let len = self.detail_lines.len();
                if len == 0 {
                    return;
                }
                let new = self.sel_detail as i32 + delta;
                self.sel_detail = new.clamp(0, len as i32 - 1) as usize;
            }
        }
    }

    /// Enter / Right arrow: context-dependent activate.
    fn activate(&mut self) {
        match self.focus {
            Focus::Services => {
                self.ensure_service_root();
                self.sel_object = 0;
                self.focus = Focus::Objects;
                self.rebuild_detail_lines();
            }
            Focus::Objects => {
                self.toggle_object_expand();
            }
            Focus::Details => self.activate_detail(),
        }
    }

    /// Right arrow: go one level deeper without ever collapsing anything.
    fn descend(&mut self) {
        match self.focus {
            Focus::Services => self.activate(),
            Focus::Objects => {
                let collapsed_with_children = self.selected_object_path().is_some_and(|p| {
                    !self.expanded.contains(&p)
                        && self
                            .selected_node()
                            .is_some_and(|n| !n.children.is_empty() || !n.loaded)
                });
                if collapsed_with_children {
                    self.toggle_object_expand();
                } else if !self.detail_lines.is_empty() {
                    self.focus = Focus::Details;
                }
            }
            Focus::Details => self.activate_detail(),
        }
    }

    fn collapse_or_back(&mut self) {
        match self.focus {
            Focus::Objects => {
                if let Some(path) = self.selected_object_path() {
                    if self.expanded.contains(&path) {
                        self.expanded.remove(&path);
                        return;
                    }
                }
                self.focus = Focus::Services;
            }
            Focus::Details => self.focus = Focus::Objects,
            Focus::Services => {}
        }
    }

    /// Introspect the selected object if that has not been attempted yet, so
    /// the details pane fills in as the cursor moves.
    fn load_selected_object(&mut self) {
        let Some(service) = self.selected_service().map(|s| s.name.clone()) else {
            return;
        };
        let Some(path) = self.selected_object_path() else {
            return;
        };
        let Some(node) = self.selected_node_mut() else {
            return;
        };
        if node.loaded || node.loading || node.error.is_some() {
            return;
        }
        node.loading = true;
        self.spawn_introspect(service, path);
    }

    fn toggle_object_expand(&mut self) {
        let Some(path) = self.selected_object_path() else {
            return;
        };
        let Some(service) = self.selected_service().map(|s| s.name.clone()) else {
            return;
        };
        let (loaded, loading, has_children) = {
            let Some(root) = self.objects.get(&service) else {
                return;
            };
            let Some(node) = find_node(root, &path) else {
                return;
            };
            (node.loaded, node.loading, !node.children.is_empty())
        };
        if !loaded && !loading {
            if let Some(root) = self.objects.get_mut(&service) {
                if let Some(node) = find_node_mut(root, &path) {
                    node.loading = true;
                }
            }
            self.expanded.insert(path.clone());
            self.spawn_introspect(service, path);
        } else if (has_children || !loaded) && !self.expanded.insert(path.clone()) {
            self.expanded.remove(&path);
        }
    }

    fn activate_detail(&mut self) {
        let Some(&line) = self.detail_lines.get(self.sel_detail) else {
            return;
        };
        match line {
            DetailLine::Header { iface } => {
                if let Some(node) = self.selected_node_mut() {
                    if let Some(i) = node.interfaces.get_mut(iface) {
                        i.collapsed = !i.collapsed;
                    }
                }
                self.rebuild_detail_lines();
            }
            DetailLine::Method { iface, method } => {
                // Pull out everything we need, then drop the tree borrow
                // before mutating input state.
                let call = (|app: &App| {
                    let service = app.selected_service()?.name.clone();
                    let path = app.selected_object_path()?;
                    let node = app.selected_node()?;
                    let iface_info = node.interfaces.get(iface)?;
                    let m = iface_info.methods.get(method)?;
                    let hint = if m.in_args.is_empty() {
                        "(no arguments — press Enter to call)".to_string()
                    } else {
                        let parts: Vec<String> = m
                            .in_args
                            .iter()
                            .map(|a| {
                                if a.name.is_empty() {
                                    a.ty.clone()
                                } else {
                                    format!("{} {}", a.ty, a.name)
                                }
                            })
                            .collect();
                        format!("args: {}", parts.join(", "))
                    };
                    let sigs = m.in_args.iter().map(|a| a.ty.clone()).collect();
                    Some((
                        service,
                        path,
                        iface_info.name.clone(),
                        m.name.clone(),
                        hint,
                        sigs,
                    ))
                })(self);
                let Some((service, path, iface_name, method_name, hint, sigs)) = call else {
                    return;
                };
                self.input.clear();
                self.set_status("", false);
                self.input_mode = InputMode::CallArgs {
                    service,
                    path,
                    iface: iface_name,
                    method: method_name,
                    hint,
                    sigs,
                };
            }
            DetailLine::Signal { .. } => {
                self.set_status("press m to monitor live signals", false);
            }
            DetailLine::Property { iface, prop } => {
                // Show the full value if we already have it, else fetch.
                let cached = self.selected_node().and_then(|n| {
                    let i = n.interfaces.get(iface)?;
                    let p = i.properties.get(prop)?;
                    let (_, v) = n
                        .prop_values
                        .get(&i.name)?
                        .iter()
                        .find(|(k, _)| *k == p.name)?;
                    Some(format!("{}.{} =\n{v}", i.name, p.name))
                });
                match cached {
                    Some(text) => self.last_result = Some((text, false)),
                    None => self.fetch_properties_for(iface),
                }
            }
        }
    }

    fn fetch_properties(&mut self) {
        // `g` fetches properties for the interface under the cursor.
        let Some(&line) = self.detail_lines.get(self.sel_detail) else {
            return;
        };
        let iface = match line {
            DetailLine::Header { iface } => Some(iface),
            DetailLine::Method { iface, .. }
            | DetailLine::Signal { iface, .. }
            | DetailLine::Property { iface, .. } => Some(iface),
        };
        if let Some(i) = iface {
            self.fetch_properties_for(i);
        }
    }

    fn fetch_properties_for(&mut self, iface_idx: usize) {
        let Some(service) = self.selected_service().map(|s| s.name.clone()) else {
            return;
        };
        let Some(path) = self.selected_object_path() else {
            return;
        };
        let Some(node) = self.selected_node() else {
            return;
        };
        let Some(iface) = node.interfaces.get(iface_idx) else {
            return;
        };
        if iface.properties.is_empty() {
            self.set_status(format!("{} has no properties", iface.name), false);
            return;
        }
        let name = iface.name.clone();
        self.set_status(format!("fetching {name} properties…"), false);
        self.spawn_props(service, path, name);
    }

    fn toggle_monitor(&mut self) {
        self.mode = match self.mode {
            Mode::Explore => {
                if !self.monitor_started {
                    self.monitor_started = true;
                    let client = self.client.clone();
                    let tx = self.tx.clone();
                    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel::<SignalEvent>();
                    let tx_setup = tx.clone();
                    tokio::spawn(async move {
                        let res = client
                            .spawn_signal_monitor(sig_tx)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = tx_setup.send(BusEvent::MonitorStarted(res));
                    });
                    // Forward raw signal events into the app's event channel.
                    tokio::spawn(async move {
                        while let Some(ev) = sig_rx.recv().await {
                            if tx.send(BusEvent::Signal(ev)).is_err() {
                                break;
                            }
                        }
                    });
                }
                Mode::Monitor
            }
            Mode::Monitor => Mode::Explore,
        };
    }

    // called from the event loop
    fn start_services(&self) {
        self.spawn_services();
    }
}

fn flatten_node(
    node: &ObjectNode,
    depth: usize,
    expanded: &HashSet<String>,
    out: &mut Vec<FlatObject>,
) {
    let name = if node.path == "/" {
        "/".to_string()
    } else {
        node.path
            .rsplit('/')
            .next()
            .unwrap_or(&node.path)
            .to_string()
    };
    out.push(FlatObject {
        depth,
        name,
        path: node.path.clone(),
        loaded: node.loaded,
        loading: node.loading,
        error: node.error.is_some(),
        has_children: !node.children.is_empty(),
    });
    if expanded.contains(&node.path) {
        for c in &node.children {
            flatten_node(c, depth + 1, expanded, out);
        }
    }
}

// ---- terminal plumbing -------------------------------------------------------

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    Ok(())
}

pub async fn run(client: BusClient) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, client).await;
    restore_terminal(&mut terminal)?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: BusClient,
) -> Result<()> {
    let mut app = App::new(client);
    app.start_services();

    let mut term_events = EventStream::new();
    let mut tick = interval(Duration::from_millis(120));
    tick.tick().await; // first tick is immediate

    while app.running {
        terminal.draw(|f| ui::draw(f, &app))?;

        tokio::select! {
            maybe_event = term_events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => app.on_key(key),
                    Some(Ok(Event::Paste(text))) => app.on_paste(&text),
                    Some(Ok(Event::Resize(_, _))) => {}
                    _ => {}
                }
            }
            maybe_bus = app.rx.recv() => {
                if let Some(ev) = maybe_bus {
                    app.handle_bus_event(ev);
                }
            }
            _ = tick.tick() => {}
        }
    }
    Ok(())
}
