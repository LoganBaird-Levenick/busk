//! Thin async wrapper around zbus for everything busk needs from D-Bus.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::time::timeout;
use zbus::names::BusName;
use zbus::zvariant::{self, ObjectPath, Str, Structure, Value};
use zbus::{fdo, Connection, MatchRule, MessageStream, MessageType};

use crate::model::*;

pub const INTROSPECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub struct BusClient {
    conn: Connection,
    pub system: bool,
}

impl BusClient {
    pub async fn connect(system: bool) -> Result<Self> {
        let conn = if system {
            Connection::system().await
        } else {
            Connection::session().await
        }
        .context("failed to connect to D-Bus")?;
        Ok(Self { conn, system })
    }

    /// List every name currently on the bus, well-known names first.
    pub async fn list_services(&self) -> Result<Vec<ServiceEntry>> {
        let proxy = fdo::DBusProxy::new(&self.conn).await?;
        let names = proxy.list_names().await?;
        let mut well_known = Vec::new();
        let mut unique = Vec::new();
        for n in names {
            let name = n.as_str().to_string();
            let wk = !name.starts_with(':');
            let owner = if wk {
                proxy
                    .get_name_owner(BusName::try_from(name.as_str())?)
                    .await
                    .ok()
                    .map(|o| o.as_str().to_string())
            } else {
                Some(name.clone())
            };
            let entry = ServiceEntry {
                name,
                owner,
                well_known: wk,
            };
            if wk {
                well_known.push(entry);
            } else {
                unique.push(entry);
            }
        }
        well_known.sort_by(|a, b| a.name.cmp(&b.name));
        unique.sort_by_key(|e| unique_sort_key(&e.name));
        well_known.extend(unique);
        Ok(well_known)
    }

    /// Introspect one object path: returns (interfaces, child node names).
    pub async fn introspect(
        &self,
        dest: &str,
        path: &str,
    ) -> Result<(Vec<InterfaceInfo>, Vec<String>)> {
        let proxy = fdo::IntrospectableProxy::builder(&self.conn)
            .destination(dest)?
            .path(path)?
            .build()
            .await
            .with_context(|| format!("creating introspectable proxy for {dest}{path}"))?;
        let xml = timeout(INTROSPECT_TIMEOUT, proxy.introspect())
            .await
            .map_err(|_| anyhow!("introspection of {dest}{path} timed out"))??;
        parse_introspection(&xml)
    }

    /// `org.freedesktop.DBus.Properties.GetAll` for one interface.
    pub async fn get_all_properties(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
    ) -> Result<Vec<(String, String)>> {
        let reply = timeout(
            INTROSPECT_TIMEOUT,
            self.conn.call_method(
                Some(dest),
                path,
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &(iface,),
            ),
        )
        .await
        .map_err(|_| anyhow!("GetAll {iface} timed out"))??;
        let body = reply.body();
        let map: HashMap<String, Value> = body.deserialize()?;
        let mut out: Vec<(String, String)> =
            map.into_iter().map(|(k, v)| (k, fmt_value(&v))).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Dynamically call a method. `args` are positional D-Bus values.
    pub async fn call_method(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: Vec<Value<'static>>,
    ) -> Result<String> {
        let reply = if args.is_empty() {
            timeout(
                INTROSPECT_TIMEOUT,
                self.conn
                    .call_method(Some(dest), path, Some(iface), method, &()),
            )
            .await
            .map_err(|_| anyhow!("call timed out"))??
        } else {
            let mut builder = zvariant::StructureBuilder::new();
            for a in args {
                builder = builder.append_field(a);
            }
            let body = builder.build();
            timeout(
                INTROSPECT_TIMEOUT,
                self.conn
                    .call_method(Some(dest), path, Some(iface), method, &body),
            )
            .await
            .map_err(|_| anyhow!("call timed out"))??
        };
        Ok(fmt_body(&reply))
    }

    /// Add a match rule for all signals and forward them over `tx` until the
    /// task is aborted. Spawns a detached task; call once.
    pub async fn spawn_signal_monitor(&self, tx: mpsc::UnboundedSender<SignalEvent>) -> Result<()> {
        let rule = MatchRule::builder().msg_type(MessageType::Signal).build();
        let mut stream = MessageStream::for_match_rule(rule, &self.conn, Some(4096)).await?;
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                if msg.message_type() != MessageType::Signal {
                    continue;
                }
                let header = msg.header();
                let ev = SignalEvent {
                    time: now_hms(),
                    sender: header
                        .sender()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default(),
                    destination: header
                        .destination()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default(),
                    path: header
                        .path()
                        .map(|p| p.as_str().to_string())
                        .unwrap_or_default(),
                    interface: header
                        .interface()
                        .map(|i| i.as_str().to_string())
                        .unwrap_or_default(),
                    member: header
                        .member()
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                    body: fmt_body(&msg),
                };
                if tx.send(ev).is_err() {
                    break; // app gone
                }
            }
        });
        Ok(())
    }
}

fn now_hms() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Parse introspection XML into our model types.
pub fn parse_introspection(xml: &str) -> Result<(Vec<InterfaceInfo>, Vec<String>)> {
    let node = zbus_xml::Node::from_reader(xml.as_bytes())
        .map_err(|e| anyhow!("parsing introspection XML: {e}"))?;
    let interfaces = node
        .interfaces()
        .iter()
        .map(|i| {
            let methods = i
                .methods()
                .iter()
                .map(|m| {
                    let mut in_args = Vec::new();
                    let mut out_args = Vec::new();
                    for a in m.args() {
                        let info = ArgInfo {
                            name: a.name().unwrap_or_default().to_string(),
                            ty: a.ty().to_string(),
                        };
                        match a.direction() {
                            Some(zbus_xml::ArgDirection::Out) => out_args.push(info),
                            _ => in_args.push(info),
                        }
                    }
                    MethodInfo {
                        name: m.name().to_string(),
                        in_args,
                        out_args,
                    }
                })
                .collect();
            let signals = i
                .signals()
                .iter()
                .map(|s| SignalInfo {
                    name: s.name().to_string(),
                    args: s
                        .args()
                        .iter()
                        .map(|a| ArgInfo {
                            name: a.name().unwrap_or_default().to_string(),
                            ty: a.ty().to_string(),
                        })
                        .collect(),
                })
                .collect();
            let properties = i
                .properties()
                .iter()
                .map(|p| PropertyInfo {
                    name: p.name().to_string(),
                    ty: p.ty().to_string(),
                    access: format!("{:?}", p.access()).to_lowercase(),
                })
                .collect();
            InterfaceInfo {
                name: i.name().to_string(),
                methods,
                signals,
                properties,
                collapsed: false,
            }
        })
        .collect();
    let children = node
        .nodes()
        .iter()
        .filter_map(|n| n.name().map(|s| s.to_string()))
        .collect();
    Ok((interfaces, children))
}

/// Split an argument line into tokens. Whitespace separates tokens; single or
/// double quotes group (and may yield an empty token); backslash escapes the
/// next character outside single quotes.
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                cur.push(chars.next().ok_or_else(|| anyhow!("dangling backslash"))?);
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_token = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(anyhow!("unterminated quote"));
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// Parse a command line of arguments against a method's declared input
/// signatures (one complete type per entry), e.g. `["s", "u", "as"]`.
pub fn parse_args(sigs: &[&str], input: &str) -> Result<Vec<Value<'static>>> {
    let tokens = tokenize(input)?;
    if tokens.len() != sigs.len() {
        return Err(anyhow!(
            "expected {} argument(s) ({}), got {}",
            sigs.len(),
            sigs.join(" "),
            tokens.len()
        ));
    }
    sigs.iter()
        .zip(&tokens)
        .enumerate()
        .map(|(i, (sig, raw))| {
            parse_typed(sig, raw).with_context(|| format!("argument {} ({sig})", i + 1))
        })
        .collect()
}

/// Parse one value of D-Bus type `sig`.
///
/// Basic types take their natural text form (`b` accepts true/false). Arrays
/// of basic types take comma-separated elements; any array or dict may be
/// empty (`''` or `[]`). `v` takes `sig:value`. Structs and non-empty
/// dicts/nested arrays are not supported.
fn parse_typed(sig: &str, raw: &str) -> Result<Value<'static>> {
    Ok(match sig {
        "s" => Value::Str(Str::from(raw.to_string())),
        "o" => Value::ObjectPath(ObjectPath::try_from(raw.to_string())?),
        "g" => Value::Signature(zvariant::Signature::try_from(raw.to_string())?),
        "y" => Value::U8(raw.parse()?),
        "n" => Value::I16(raw.parse()?),
        "q" => Value::U16(raw.parse()?),
        "i" => Value::I32(raw.parse()?),
        "u" => Value::U32(raw.parse()?),
        "x" => Value::I64(raw.parse()?),
        "t" => Value::U64(raw.parse()?),
        "d" => Value::F64(raw.parse()?),
        "b" => Value::Bool(match raw {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => return Err(anyhow!("expected true or false, got `{raw}`")),
        }),
        "v" => {
            let (inner_sig, inner) = raw
                .split_once(':')
                .ok_or_else(|| anyhow!("variant must look like `sig:value`, got `{raw}`"))?;
            Value::Value(Box::new(parse_typed(inner_sig, inner)?))
        }
        _ => {
            let Some(elem) = sig.strip_prefix('a') else {
                return Err(anyhow!("unsupported type `{sig}`"));
            };
            let empty = raw.is_empty() || raw == "[]";
            if let Some(entry) = elem.strip_prefix('{').and_then(|e| e.strip_suffix('}')) {
                if !empty {
                    return Err(anyhow!(
                        "only empty dicts can be entered (`''`) for `{sig}`"
                    ));
                }
                let key = entry
                    .get(..1)
                    .ok_or_else(|| anyhow!("malformed dict type `{sig}`"))?;
                return Ok(Value::Dict(zvariant::Dict::new(
                    zvariant::Signature::try_from(key.to_string())?,
                    zvariant::Signature::try_from(entry[1..].to_string())?,
                )));
            }
            let mut array = zvariant::Array::new(zvariant::Signature::try_from(elem.to_string())?);
            if !empty {
                if !matches!(
                    elem,
                    "s" | "o" | "g" | "y" | "n" | "q" | "i" | "u" | "x" | "t" | "d" | "b" | "v"
                ) {
                    return Err(anyhow!(
                        "only empty arrays can be entered (`''`) for `{sig}`"
                    ));
                }
                for part in raw.split(',') {
                    array.append(parse_typed(elem, part.trim())?)?;
                }
            }
            Value::Array(array)
        }
    })
}

/// Render every value in a message body, space-separated; `()` when empty.
pub fn fmt_body(msg: &zbus::Message) -> String {
    let body = msg.body();
    if body.signature().is_none() {
        return "()".into();
    }
    match body.deserialize::<Structure>() {
        Ok(s) if s.fields().is_empty() => "()".into(),
        Ok(s) => s
            .fields()
            .iter()
            .map(fmt_value)
            .collect::<Vec<_>>()
            .join("  "),
        Err(e) => format!("<unparseable body: {e}>"),
    }
}

/// Human-friendly rendering of a D-Bus value.
pub fn fmt_value(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("\"{s}\""),
        Value::ObjectPath(p) => p.to_string(),
        Value::Signature(s) => format!("'{s}'"),
        Value::Bool(b) => b.to_string(),
        Value::U8(n) => n.to_string(),
        Value::I16(n) => n.to_string(),
        Value::U16(n) => n.to_string(),
        Value::I32(n) => n.to_string(),
        Value::U32(n) => n.to_string(),
        Value::I64(n) => n.to_string(),
        Value::U64(n) => n.to_string(),
        Value::F64(n) => n.to_string(),
        Value::Value(inner) => fmt_value(inner),
        Value::Fd(_) => "<fd>".to_string(),
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(fmt_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Dict(d) => {
            let inner: Vec<String> = d
                .iter()
                .map(|(k, val)| format!("{}: {}", fmt_value(k), fmt_value(val)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Structure(s) => {
            let inner: Vec<String> = s.fields().iter().map(fmt_value).collect();
            format!("({})", inner.join(", "))
        }
    }
}

/// Sort `:1.9` before `:1.10`.
fn unique_sort_key(name: &str) -> Vec<u64> {
    name.trim_start_matches(':')
        .split('.')
        .map(|p| p.parse().unwrap_or(u64::MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_quotes_and_empty() {
        assert_eq!(
            tokenize(r#"a "b c" '' d\ e 'f"g'"#).unwrap(),
            vec!["a", "b c", "", "d e", "f\"g"]
        );
        assert!(tokenize("'open").is_err());
    }

    #[test]
    fn args_follow_declared_types() {
        let v = parse_args(&["s", "u", "as", "b"], r#""hello world" 7 a,b true"#).unwrap();
        assert_eq!(v.len(), 4);
        assert_eq!(fmt_value(&v[0]), "\"hello world\"");
        assert_eq!(fmt_value(&v[1]), "7");
        assert_eq!(fmt_value(&v[2]), "[\"a\", \"b\"]");
        assert_eq!(fmt_value(&v[3]), "true");
    }

    #[test]
    fn args_reject_bad_input() {
        assert!(parse_args(&["s"], "").is_err(), "missing argument");
        assert!(parse_args(&["u"], "-1").is_err());
        assert!(parse_args(&["b"], "maybe").is_err());
        assert!(parse_args(&["(ss)"], "x").is_err());
        assert!(parse_args(&["a{sv}"], "k").is_err());
    }

    #[test]
    fn empty_containers_and_variants() {
        let v = parse_args(&["a{sv}", "as", "v"], "'' [] u:5").unwrap();
        assert_eq!(fmt_value(&v[0]), "{}");
        assert_eq!(fmt_value(&v[1]), "[]");
        assert_eq!(fmt_value(&v[2]), "5");
    }

    #[test]
    fn unique_names_sort_numerically() {
        let mut v = vec![":1.10", ":1.9", ":1.100", ":1.2"];
        v.sort_by_key(|n| unique_sort_key(n));
        assert_eq!(v, vec![":1.2", ":1.9", ":1.10", ":1.100"]);
    }

    #[test]
    fn introspection_xml() {
        let xml = r#"<node>
  <interface name="org.example.Foo">
    <method name="Bar"><arg name="x" type="s" direction="in"/><arg name="y" type="u" direction="out"/></method>
    <signal name="Baz"><arg name="z" type="i"/></signal>
    <property name="P" type="b" access="readwrite"/>
  </interface>
  <node name="child"/>
</node>"#;
        let (ifaces, children) = parse_introspection(xml).unwrap();
        assert_eq!(children, vec!["child"]);
        let i = &ifaces[0];
        assert_eq!(i.methods[0].in_args[0].ty, "s");
        assert_eq!(i.methods[0].out_args[0].ty, "u");
        assert_eq!(i.signals[0].args[0].ty, "i");
        assert_eq!(i.properties[0].access, "readwrite");
    }
}
