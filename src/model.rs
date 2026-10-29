//! Core data structures shared between the D-Bus client, app state, and UI.

use std::collections::HashMap;

/// A name currently owned on the bus.
#[derive(Debug, Clone)]
pub struct ServiceEntry {
    /// Well-known name (e.g. `org.freedesktop.Notifications`) or the unique
    /// name (`:1.42`) when the service exports no well-known name.
    pub name: String,
    /// Unique connection name owning `name` (e.g. `:1.42`), when known.
    pub owner: Option<String>,
    /// True if `name` is a well-known name (not `:1.x`).
    pub well_known: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ArgInfo {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, Default)]
pub struct MethodInfo {
    pub name: String,
    pub in_args: Vec<ArgInfo>,
    pub out_args: Vec<ArgInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct SignalInfo {
    pub name: String,
    pub args: Vec<ArgInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct PropertyInfo {
    pub name: String,
    pub ty: String,
    pub access: String, // "read", "write", "readwrite"
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceInfo {
    pub name: String,
    pub methods: Vec<MethodInfo>,
    pub signals: Vec<SignalInfo>,
    pub properties: Vec<PropertyInfo>,
    /// UI state: section collapsed in the details pane.
    pub collapsed: bool,
}

/// A node in a service's object tree.
#[derive(Debug, Default)]
pub struct ObjectNode {
    /// Full object path, e.g. `/org/freedesktop/DBus`.
    pub path: String,
    pub children: Vec<ObjectNode>,
    pub interfaces: Vec<InterfaceInfo>,
    /// Introspection has completed for this node.
    pub loaded: bool,
    /// An introspection request is in flight.
    pub loading: bool,
    /// Last introspection error, if any.
    pub error: Option<String>,
    /// Cached `GetAll` results per interface name.
    pub prop_values: HashMap<String, Vec<(String, String)>>,
}

impl ObjectNode {
    pub fn new(path: String) -> Self {
        Self {
            path,
            ..Default::default()
        }
    }

    pub fn child(&self, path: &str) -> Option<&ObjectNode> {
        for c in &self.children {
            if c.path == path {
                return Some(c);
            }
            if let Some(found) = c.child(path) {
                return Some(found);
            }
        }
        None
    }

    pub fn child_mut(&mut self, path: &str) -> Option<&mut ObjectNode> {
        for c in &mut self.children {
            if c.path == path {
                return Some(c);
            }
            if let Some(found) = c.child_mut(path) {
                return Some(found);
            }
        }
        None
    }
}

/// Join a child node name onto a parent object path.
pub fn join_path(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

/// Find a node by full path, including the root itself.
pub fn find_node<'a>(root: &'a ObjectNode, path: &str) -> Option<&'a ObjectNode> {
    if root.path == path {
        Some(root)
    } else {
        root.child(path)
    }
}

/// Mutable variant of [`find_node`].
pub fn find_node_mut<'a>(root: &'a mut ObjectNode, path: &str) -> Option<&'a mut ObjectNode> {
    if root.path == path {
        Some(root)
    } else {
        root.child_mut(path)
    }
}

/// One received D-Bus signal, for the monitor view.
#[derive(Debug, Clone)]
pub struct SignalEvent {
    pub time: String,
    pub sender: String,
    pub destination: String,
    pub path: String,
    pub interface: String,
    pub member: String,
    pub body: String,
}
