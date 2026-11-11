//! `io.github.design_nexus.nyx.Bar1` on the session bus, as the GTK bar serves it, for
//! `nyx plugin list` and friends. The test methods reach the interface as UI events.

use crate::host::{self, Shared};
use gio::prelude::*;
use serde_json::json;
use std::rc::Rc;

pub const PATH: &str = "/io/github/design_nexus/nyx/Bar";
pub const INTERFACE: &str = "io.github.design_nexus.nyx.Bar1";

const XML: &str = r#"<node>
  <interface name="io.github.design_nexus.nyx.Bar1">
    <method name="ListPlugins"><arg type="s" name="json" direction="out"/></method>
    <method name="Reload"/>
    <method name="RestartPlugin"><arg type="s" name="id" direction="in"/><arg type="b" name="found" direction="out"/></method>
    <method name="TestClick"><arg type="s" name="plugin" direction="in"/><arg type="s" name="item" direction="in"/><arg type="u" name="button" direction="in"/><arg type="b" name="found" direction="out"/></method>
    <method name="TestClose"/>
  </interface>
</node>"#;

pub fn serve(conn: &gio::DBusConnection, s: &Rc<Shared>) -> Result<gio::RegistrationId, glib::Error> {
    let info = gio::DBusNodeInfo::for_xml(XML)?.lookup_interface(INTERFACE).expect("Bar1");
    let ws = Rc::downgrade(s);
    conn.register_object(PATH, &info)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let Some(s) = ws.upgrade() else {
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "the bar is closing");
                return;
            };
            match method {
                "ListPlugins" => invocation.return_value(Some(&(host::list(&s).to_string(),).to_variant())),
                "Reload" => {
                    host::sync_plugins(&s);
                    invocation.return_value(None);
                }
                "RestartPlugin" => {
                    let id = params.get::<(String,)>().map(|(i,)| i).unwrap_or_default();
                    invocation.return_value(Some(&(host::restart(&s, &id),).to_variant()));
                }
                "TestClick" | "TestClose" if std::env::var_os("NYX_BAR_DEBUG").is_none() => {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "test methods need NYX_BAR_DEBUG")
                }
                "TestClick" => {
                    let (plugin, item, button) = params.get::<(String, String, u32)>().unwrap_or_default();
                    s.link.emit("TestClick", json!({"plugin": plugin, "item": item, "button": button}));
                    invocation.return_value(Some(&(true,).to_variant()));
                }
                "TestClose" => {
                    s.link.emit("TestClose", json!({}));
                    invocation.return_value(None);
                }
                _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
            }
        })
        .build()
}
