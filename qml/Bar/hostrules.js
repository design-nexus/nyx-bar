.pragma library

// The bar's own classes for plugin views, as the GTK bar's style.css has them, in the
// form a plugin's compiled style.css takes (Nyx.PluginView's style.js). They come first;
// a plugin's rules of the same weight win.

function c(t, a) {
    const out = { t: t }
    if (a !== undefined)
        out.a = a
    return out
}

// `sel` as "&.bar-item:hover" / ".chip.warning" / "button.flat": compounds by spaces.
function compound(s) {
    const out = { type: null, classes: [], not: [], hover: false, root: false }
    let rest = s
    if (rest[0] === "&") {
        out.root = true
        rest = rest.slice(1)
    } else {
        const m = rest.match(/^[a-z]+/)
        if (m) {
            out.type = m[0]
            rest = rest.slice(m[0].length)
        }
    }
    const parts = rest.match(/\.[\w-]+|:hover|:not\(\.[\w-]+\)/g) || []
    for (const p of parts) {
        if (p === ":hover")
            out.hover = true
        else if (p.startsWith(":not"))
            out.not.push(p.slice(6, -1))
        else
            out.classes.push(p.slice(1))
    }
    return out
}

function rule(sel, props) {
    const comps = sel.split(" ").map(compound)
    let spec = 0
    for (const x of comps)
        spec += x.classes.length + x.not.length + (x.hover ? 1 : 0) + (x.type ? 1 : 0) + (x.root ? 1 : 0)
    return { sel: comps, spec: spec, props: props }
}

const rules = [
    // Text.
    rule(".accent", { color: c("accent") }),
    rule(".success", { color: c("success") }),
    rule(".warning", { color: c("warning") }),
    rule(".danger", { color: c("danger") }),
    rule(".dim", { color: c("dim_text") }),
    rule(".tertiary", { color: c("text_tertiary") }),
    rule(".active", { color: c("accent"), "font-weight": 600 }),
    rule(".urgent", { color: c("danger"), "font-weight": 600 }),
    rule(".strong", { "font-weight": 600 }),
    rule(".small", { "font-size": 12 }),
    rule(".xs", { "font-size": 11 }),
    rule(".big", { "font-size": 20, "font-weight": 600 }),
    rule(".huge", { "font-size": 28, "font-weight": 600 }),
    rule(".caps", { "font-size": 11, "font-weight": 600, "letter-spacing": 0.66, color: c("text_tertiary") }),
    // A chip beside a label: the clock's next event, an alert.
    rule(".chip", { padding: [0, 7, 0, 7], "min-height": 18, "border-radius": 9, "font-size": 11.5, "font-weight": 500, "background-color": c("accent", 0.16) }),
    rule(".chip.warning", { color: c("warning"), "background-color": c("warning", 0.2) }),
    rule(".chip.danger", { color: c("danger"), "background-color": c("danger", 0.16) }),
    // A key cap: the keyboard layout's code.
    rule(".kbd", { "min-width": 14, "min-height": 17, padding: [0, 4, 0, 4], "border-radius": 5, "font-size": 10.5, "font-weight": 700, ring: { w: 1.3, c: c("text", 0.35) } }),
    // Popovers.
    rule(".card", { padding: [8, 8, 8, 8], "border-radius": 12, "background-color": c("text", 0.035) }),
    rule(".selected", { "background-color": c("accent", 0.13), "border-radius": 10 }),
    rule(".plugin-row", { padding: [6, 8, 6, 8], "border-radius": 9 }),
    rule(".plugin-row:hover", { "background-color": c("text", 0.06) }),
    rule(".pfoot", { "margin-top": 4, padding: [10, 6, 2, 6], "border-top": { w: 1, c: c("text", 0.09) } }),
    // Buttons, as the theme draws them in GTK.
    rule("button", { padding: [0, 10, 0, 10], "min-height": 28, "border-radius": 7, "font-size": 12.5, "font-weight": 500, "background-color": c("text", 0.08) }),
    rule("button:hover", { "background-color": c("text", 0.12) }),
    rule("button.flat", { "background-color": { none: true }, padding: [0, 6, 0, 6] }),
    rule("button.flat:hover", { "background-color": c("text", 0.08) }),
    rule("button.suggested-action", { color: c("on_accent"), "background-color": c("accent") }),
    rule("button.link", { padding: [0, 0, 0, 0], "min-height": 0, "background-color": { none: true }, color: c("accent"), "font-size": 12.5, "font-weight": 500 }),
    // The bar's items light on hover only; an open popover or menu doesn't tint them.
    rule("&.bar-item:hover", { "background-color": c("text", 0.07) }),
    rule("&.bar-item.recording", { color: c("danger"), "background-color": c("danger", 0.12) }),
]
