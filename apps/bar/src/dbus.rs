//! `io.github.design_nexus.nyx.Bar1` on the session bus, for `nyx plugin list` and
//! friends: what runs, what it uses, and reloading.

use crate::bar::{self, Shared};
use gtk::gio;
use gtk::prelude::*;
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

pub struct Registration {
    _id: gio::RegistrationId,
}

pub fn serve(conn: &gio::DBusConnection, s: &Rc<Shared>) -> Result<Registration, gtk::glib::Error> {
    let info = gio::DBusNodeInfo::for_xml(XML)?.lookup_interface(INTERFACE).expect("Bar1");
    let ws = Rc::downgrade(s);
    let id = conn
        .register_object(PATH, &info)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let Some(s) = ws.upgrade() else {
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "the bar is closing");
                return;
            };
            match method {
                "ListPlugins" => {
                    let json = bar::list(&s).to_string();
                    invocation.return_value(Some(&(json,).to_variant()));
                }
                "Reload" => {
                    bar::sync_plugins(&s);
                    invocation.return_value(None);
                }
                "RestartPlugin" => {
                    let id = params.get::<(String,)>().map(|(i,)| i).unwrap_or_default();
                    let found = bar::restart(&s, &id);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                "TestClick" | "TestClose" if std::env::var_os("NYX_BAR_DEBUG").is_none() => {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "test methods need NYX_BAR_DEBUG")
                }
                "TestClick" => {
                    let (plugin, item, button) = params.get::<(String, String, u32)>().unwrap_or_default();
                    let found = bar::test_click(&s, &plugin, &item, button);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                "TestClose" => {
                    bar::test_close(&s);
                    invocation.return_value(None);
                }
                _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
            }
        })
        .build()?;
    Ok(Registration { _id: id })
}
