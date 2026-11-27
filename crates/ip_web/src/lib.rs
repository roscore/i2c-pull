//! Browser front end: breadboard canvas, verdict banner, animated scope strip, A/B compare and the
//! Details calculator. All logic lives in `ip_core`; this crate only reads the DOM and draws.
#[allow(dead_code)]
mod gl;
#[allow(dead_code)]
mod math;
mod scene;
mod view2d;
mod view3d;
use scene::issue_kind;
use view2d::{board_scale, draw_board, drop_slot, hit_test};
use ip_core::*;
use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d as Ctx, Document, Element, Event, HtmlCanvasElement};
use web_sys::{HtmlElement, HtmlInputElement, HtmlSelectElement, KeyboardEvent, MouseEvent, PointerEvent};

const CB_X: (f64, f64) = (10.0, 600.0);
const RP_Y: (f64, f64) = (100.0, 100_000.0);
const C_TEXT: &str = "#9aa7bd";
const C_GRID: &str = "rgba(154,167,189,0.14)";
const C_ACC: &str = "#4fd1c5";
const C_WARN: &str = "#f6c453";
const C_BAD: &str = "#ff6b6b";
const C_OK: &str = "#5ee38f";
const C_GHOST: &str = "rgba(190,196,210,0.45)";
/// Scope speed: clock periods per real second.
const SCOPE_RATE: f64 = 0.8;
const E12_PICK: [f64; 21] = [
    1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2, 10.0, 12.0, 15.0, 18.0, 22.0, 27.0, 33.0, 39.0, 47.0,
];
const BUS_FIELDS: [Field; 4] = [Field::PfPerCm, Field::ExtraPf, Field::Vol, Field::Iol];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Mcu,
    Part(usize),
}

#[derive(Clone, Copy)]
struct Drag {
    kind: Kind,
    x0: f64,
    y0: f64,
    active: bool,
    slot: Option<usize>,
}

struct App {
    en: bool,
    board: Board,
    v: Verdict,
    pinned: Option<Board>,
    drag: Option<Drag>,
    hover: Option<Hit>,
    sel: Option<Hit>,
    tapped: bool,
    wave_cursor: Option<f64>,
    plot_hover: Option<(f64, f64)>,
    anim_t: f64,
    motion: bool,
    /// The WebGL2 board is active (false = 2D canvas fallback).
    three: bool,
}

impl App {
    fn new() -> App {
        let board = presets().swap_remove(0).2;
        let v = board.verdict();
        App {
            en: false,
            board,
            v,
            pinned: None,
            drag: None,
            hover: None,
            sel: None,
            tapped: false,
            wave_cursor: None,
            plot_hover: None,
            anim_t: 0.0,
            motion: true,
            three: false,
        }
    }
}

thread_local! { static APP: RefCell<App> = RefCell::new(App::new()); }

fn with<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

fn doc() -> Document {
    web_sys::window().and_then(|w| w.document()).expect("document")
}
fn el(id: &str) -> Option<Element> {
    doc().get_element_by_id(id)
}
fn input(id: &str) -> Option<HtmlInputElement> {
    el(id).and_then(|e| e.dyn_into().ok())
}
fn set_text(id: &str, s: &str) {
    if let Some(e) = el(id) {
        e.set_text_content(Some(s));
    }
}
fn val(id: &str) -> String {
    input(id).map(|i| i.value()).unwrap_or_default()
}
fn set_val(id: &str, v: &str) {
    if let Some(i) = input(id) {
        i.set_value(v);
    }
}
fn select(id: &str) -> Option<HtmlSelectElement> {
    el(id).and_then(|e| e.dyn_into().ok())
}
fn tr<'a>(en: bool, ko: &'a str, eng: &'a str) -> &'a str {
    if en {
        eng
    } else {
        ko
    }
}
fn on<E: JsCast + 'static, T: AsRef<web_sys::EventTarget>>(target: &T, ev: &str, f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(Event)>::new({
        let mut f = f;
        move |e: Event| {
            if let Ok(e) = e.dyn_into::<E>() {
                f(e)
            }
        }
    });
    let _ = target
        .as_ref()
        .add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref());
    cb.forget();
}
fn set_class(id: &str, cls: &str, on: bool) {
    if let Some(e) = el(id) {
        let _ = e.class_list().toggle_with_force(cls, on);
    }
}

fn sig(x: f64) -> String {
    if !x.is_finite() {
        return "∞".into();
    }
    let d = if x.abs() >= 100.0 { 0 } else if x.abs() >= 10.0 { 1 } else { 2 };
    format!("{x:.d$}")
}
fn fmt_r(o: f64) -> String {
    if !o.is_finite() {
        "∞".into()
    } else if o >= 1e6 {
        format!("{} MΩ", sig(o / 1e6))
    } else if o >= 1e3 {
        format!("{} kΩ", sig(o / 1e3))
    } else {
        format!("{} Ω", sig(o))
    }
}
fn fmt_t(ns: f64) -> String {
    if !ns.is_finite() {
        "∞".into()
    } else if ns.abs() >= 1000.0 {
        format!("{} µs", sig(ns / 1e3))
    } else {
        format!("{} ns", sig(ns))
    }
}
fn fmt_f(hz: f64) -> String {
    if hz >= 1e6 {
        format!("{} MHz", sig(hz / 1e6))
    } else {
        format!("{} kHz", sig(hz / 1e3))
    }
}
fn fmt_in(x: f64) -> String {
    if !x.is_finite() {
        return "∞".into();
    }
    let x = x + 0.0; // normalise -0.0
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
fn st_class(s: Status) -> &'static str {
    match s {
        Status::Works => "works",
        Status::Unstable => "unstable",
        Status::Fails => "fails",
    }
}
fn err_text(en: bool, e: NumErr) -> String {
    match e {
        NumErr::Empty => tr(en, "값을 입력하세요", "Enter a value").into(),
        NumErr::NotNumber => tr(en, "숫자가 아닙니다", "Not a number").into(),
        NumErr::NotInteger => tr(en, "정수만 입력", "Whole numbers only").into(),
        NumErr::OutOfRange(a, b) => format!("{} {}–{}", tr(en, "허용 범위", "Allowed"), fmt_in(a), fmt_in(b)),
    }
}

// ---------- state changes ----------

/// Re-run the verdict after any board change and redraw.
fn update() {
    with(|a| a.v = a.board.verdict());
    render();
}

fn flash_msg(s: &str) {
    set_text("board-msg", s);
}

fn add_kind(kind: Kind, slot: Option<usize>) {
    let mut p = kind.part();
    if kind == Kind::PullUp {
        let r = select("pick-r").and_then(|s| s.value().parse().ok()).unwrap_or(4.7);
        p = p.with_r(r);
    }
    if kind == Kind::Wire {
        let l = select("pick-len").and_then(|s| s.value().parse().ok()).unwrap_or(10.0);
        p = p.with_len(l);
    }
    let (ok, en) = with(|a| {
        a.sel = None;
        (a.board.add(p, slot), a.en)
    });
    flash_msg(if ok { "" } else { tr(en, "보드가 꽉 찼어요 (부품 최대 8개)", "The board is full (8 parts max)") });
    rebuild_parts();
    update();
}

fn remove_hit(h: Hit) {
    with(|a| {
        match h {
            Hit::Mcu => a.board.mcu = None,
            Hit::Part(i) => a.board.remove(i),
        }
        a.hover = None;
        a.sel = None;
    });
    flash_msg("");
    hide_tip();
    rebuild_parts();
    update();
}

fn apply_board(b: Board) {
    with(|a| {
        a.board = b;
        a.sel = None;
        a.hover = None;
    });
    sync_bus_inputs();
    rebuild_parts();
    flash_msg("");
    update();
}

fn sync_bus_inputs() {
    let bus = with(|a| a.board.bus);
    set_val("pf_per_cm", &fmt_in(bus.pf_per_cm));
    set_val("extra_pf", &fmt_in(bus.stray_pf));
    set_val("vol", &fmt_in(bus.vol));
    set_val("iol", &fmt_in(bus.iol_ma));
    if let Some(s) = select("speed") {
        s.set_selected_index(bus.speed as i32);
    }
    for f in BUS_FIELDS {
        show_err(f.id(), "");
    }
}

fn show_err(id: &str, msg: &str) {
    set_text(&format!("e-{id}"), msg);
    if let Some(i) = input(id) {
        let _ = i.set_attribute("aria-invalid", if msg.is_empty() { "false" } else { "true" });
    }
}

/// Read the editable bus fields; invalid ones show an inline message and keep the last good value.
fn read_bus() {
    let en = with(|a| a.en);
    let mut ok = true;
    let mut vals = [0.0; 4];
    for (i, f) in BUS_FIELDS.iter().enumerate() {
        match parse_field(*f, &val(f.id())) {
            Ok(x) => {
                vals[i] = x;
                show_err(f.id(), "");
            }
            Err(e) => {
                ok = false;
                show_err(f.id(), &err_text(en, e));
            }
        }
    }
    let sp = Speed::from_index(select("speed").map(|s| s.selected_index().max(0) as usize).unwrap_or(0));
    with(|a| {
        let old = a.board.bus.speed;
        a.board.bus.speed = sp;
        // Swap IOL default with the speed mode unless the user edited it.
        if old != sp && ok && vals[3] == old.default_iol_ma() {
            vals[3] = sp.default_iol_ma();
            set_val("iol", &fmt_in(vals[3]));
        }
        if ok {
            a.board.bus.pf_per_cm = vals[0];
            a.board.bus.stray_pf = vals[1];
            a.board.bus.vol = vals[2];
            a.board.bus.iol_ma = vals[3];
        }
    });
    set_class("app", "stale", !ok);
    update();
}

// ---------- palette ----------

fn part_sub(k: Kind, en: bool) -> String {
    let p = k.part();
    let pu = |p: &Part| match p.pullup_ohm() {
        Some(o) => format!("{} {}", tr(en, "풀업", "pull-up"), fmt_ohm(o)),
        None => tr(en, "풀업 없음", "no pull-up").into(),
    };
    match k {
        Kind::Uno | Kind::Esp32 | Kind::RasPi => {
            let v = format!("{}V {}", k.logic_v().unwrap_or(3.3), tr(en, "로직", "logic"));
            if p.pullup_ohm().is_some() { format!("{v} · {}", pu(&p)) } else { v }
        }
        Kind::PullUp => tr(en, "값 고르기 ↓", "pick a value ↓").into(),
        Kind::Wire => tr(en, "길이 고르기 ↓", "pick a length ↓").into(),
        Kind::LevelShifter => format!("5V↔3.3V · {}", pu(&p)),
        _ => {
            let mut s = pu(&p);
            if let Some(a) = p.addr {
                s += &format!(" · 0x{a:02X}");
            }
            if !p.five_v_ok {
                s += tr(en, " · 3.3V 전용", " · 3.3 V only");
            }
            s
        }
    }
}

fn palette_html() -> String {
    let both = |ko: &str, en: &str| format!("<span class=\"ko\">{ko}</span><span class=\"en\">{en}</span>");
    let btn = |k: Kind| {
        format!(
            "<button type=\"button\" class=\"pt\" data-kind=\"{}\"><b>{}</b><small>{}</small></button>",
            k.id(),
            both(k.name(false), k.name(true)),
            both(&part_sub(k, false), &part_sub(k, true))
        )
    };
    let groups: [(&str, &str, &[Kind]); 3] = [
        ("MCU 보드", "MCU board", &[Kind::Uno, Kind::Esp32, Kind::RasPi]),
        ("센서 모듈", "Sensor modules", &[Kind::Bme280, Kind::Mpu6050, Kind::Ssd1306, Kind::Vl53l0x, Kind::Generic10k, Kind::BareChip]),
        ("배선 · 부품", "Wiring & parts", &[Kind::PullUp, Kind::Wire, Kind::LevelShifter]),
    ];
    let mut h = String::new();
    for (ko, en, kinds) in groups {
        h += &format!("<section class=\"card\"><h2>{}</h2><div class=\"pgrid\">", both(ko, en));
        for k in kinds {
            match k {
                Kind::PullUp => {
                    let opts: String = E12_PICK
                        .iter()
                        .map(|v| format!("<option value=\"{v}\"{}>{}</option>", if *v == 4.7 { " selected" } else { "" }, fmt_ohm(v * 1e3)))
                        .collect();
                    h += &format!("<div class=\"pick\">{}<select id=\"pick-r\" aria-label=\"pull-up value\">{opts}</select></div>", btn(*k));
                }
                Kind::Wire => {
                    let opts: String = WIRE_LENGTHS_CM
                        .iter()
                        .map(|v| format!("<option value=\"{v}\"{}>{v} cm</option>", if *v == 10.0 { " selected" } else { "" }))
                        .collect();
                    h += &format!("<div class=\"pick\">{}<select id=\"pick-len\" aria-label=\"wire length\">{opts}</select></div>", btn(*k));
                }
                _ => h += &btn(*k),
            }
        }
        h += "</div></section>";
    }
    h
}

// ---------- parts table (Details) ----------

fn rebuild_parts() {
    let html = with(|a| {
        let en = a.en;
        let mut h = format!(
            "<thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>5V</th><th>{}</th><th></th></tr></thead><tbody>",
            tr(en, "부품", "Part"),
            tr(en, "풀업 kΩ", "Pull-up kΩ"),
            tr(en, "핀 pF", "Pin pF"),
            tr(en, "주소", "Address"),
            tr(en, "길이", "Length")
        );
        let rows = a.board.mcu.iter().map(|p| ("m".to_string(), p)).chain(a.board.parts.iter().enumerate().map(|(i, p)| (i.to_string(), p)));
        for (i, p) in rows {
            let k = p.kind;
            let inp = |f: &str, v: String, label: &str| {
                format!("<td><input data-i=\"{i}\" data-f=\"{f}\" value=\"{v}\" inputmode=\"decimal\" autocomplete=\"off\" aria-label=\"{label}\"></td>")
            };
            let dash = "<td>—</td>".to_string();
            let r = if k == Kind::Wire { dash.clone() } else { inp("r", if p.r_kohm > 0.0 { fmt_in(p.r_kohm) } else { String::new() }, "pull-up kOhm") };
            let pf = if matches!(k, Kind::Wire | Kind::PullUp) { dash.clone() } else { inp("pf", fmt_in(p.pin_pf), "pin pF") };
            let (addr, v5) = if k.is_target() {
                (
                    inp("addr", p.addr.map(|a| format!("0x{a:02X}")).unwrap_or_default(), "address"),
                    format!("<td><input type=\"checkbox\" data-i=\"{i}\" data-f=\"v5\"{} aria-label=\"5 V tolerant\"></td>", if p.five_v_ok { " checked" } else { "" }),
                )
            } else {
                (dash.clone(), dash.clone())
            };
            let len = if k == Kind::Wire {
                let opts: String = WIRE_LENGTHS_CM
                    .iter()
                    .map(|v| format!("<option value=\"{v}\"{}>{v} cm</option>", if *v == p.len_cm { " selected" } else { "" }))
                    .collect();
                format!("<td><select data-i=\"{i}\" data-f=\"len\" aria-label=\"length\">{opts}</select></td>")
            } else {
                dash
            };
            h += &format!(
                "<tr><th>{}</th>{r}{pf}{addr}{v5}{len}<td><button type=\"button\" data-rm=\"{i}\" aria-label=\"remove\">×</button></td></tr>",
                k.name(en)
            );
        }
        h + "</tbody>"
    });
    if let Some(t) = el("parts") {
        t.set_inner_html(&html);
    }
}

fn on_part_input(t: &Element) {
    let (Some(i), Some(f)) = (t.get_attribute("data-i"), t.get_attribute("data-f")) else { return };
    let en = with(|a| a.en);
    let value = t.clone().dyn_into::<HtmlInputElement>().map(|x| x.value()).ok().or_else(|| t.clone().dyn_into::<HtmlSelectElement>().ok().map(|s| s.value())).unwrap_or_default();
    let checked = t.clone().dyn_into::<HtmlInputElement>().map(|x| x.checked()).unwrap_or(false);
    let kind = with(|a| if i == "m" { a.board.mcu.map(|p| p.kind) } else { i.parse::<usize>().ok().and_then(|n| a.board.parts.get(n).map(|p| p.kind)) });
    let Some(kind) = kind else { return };
    // Parse first, then apply, so a bad value never reaches the model.
    enum Change {
        R(f64),
        Pf(f64),
        Addr(Option<u8>),
        V5(bool),
        Len(f64),
    }
    let res: Result<Change, NumErr> = match f.as_str() {
        "r" if value.trim().is_empty() || value.trim() == "0" => {
            if kind == Kind::PullUp { Err(NumErr::Empty) } else { Ok(Change::R(0.0)) }
        }
        "r" => parse_field(Field::RExtra, &value).map(Change::R),
        "pf" => parse_field(Field::PinPf, &value).map(Change::Pf),
        "addr" => parse_addr(&value).map(Change::Addr),
        "v5" => Ok(Change::V5(checked)),
        "len" => value.parse().map(Change::Len).map_err(|_| NumErr::NotNumber),
        _ => return,
    };
    match res {
        Ok(c) => {
            let _ = t.set_attribute("aria-invalid", "false");
            let _ = t.remove_attribute("title");
            with(|a| {
                let p = if i == "m" { a.board.mcu.as_mut() } else { i.parse::<usize>().ok().and_then(|n| a.board.parts.get_mut(n)) };
                if let Some(p) = p {
                    match c {
                        Change::R(v) => p.r_kohm = v,
                        Change::Pf(v) => p.pin_pf = v,
                        Change::Addr(v) => p.addr = v,
                        Change::V5(v) => p.five_v_ok = v,
                        Change::Len(v) => p.len_cm = v,
                    }
                }
            });
            update();
        }
        Err(e) => {
            let _ = t.set_attribute("aria-invalid", "true");
            let _ = t.set_attribute("title", &err_text(en, e));
        }
    }
}

// ---------- render ----------

fn render() {
    APP.with(|a| {
        let Ok(app) = a.try_borrow() else { return };
        let en = app.en;
        let v = &app.v;
        let (reason, fix) = v.text(en);
        set_text("v-status", v.status.label(en));
        set_text("v-reason", &reason);
        set_text("v-fix", &fix);
        if let Some(e) = el("verdict") {
            e.set_class_name(&format!("verdict {}", st_class(v.status)));
        }
        let b = &app.board;
        set_text(
            "board-sum",
            &format!(
                "{} · {} {} · {} {} cm",
                b.mcu.map(|m| m.kind.name(en)).unwrap_or(tr(en, "MCU 없음", "no MCU")),
                tr(en, "부품", "parts"),
                b.parts.len(),
                tr(en, "선 합계", "wire total"),
                fmt_in(b.wire_cm())
            ),
        );
        for (i, (ko, eng, _)) in presets().iter().enumerate() {
            set_text(&format!("preset{i}"), tr(en, ko, eng));
        }
        set_text("pin", if app.pinned.is_some() { tr(en, "저장 해제", "Clear saved") } else { tr(en, "이 회로 저장", "Save this board") });
        if app.three {
            view3d::panel(&app);
            view3d::request();
        } else {
            draw_board(&app);
        }
        draw_scope(&app);
        render_compare(&app);
        render_details(&app);
    });
}

fn render_compare(app: &App) {
    let en = app.en;
    let Some(pb) = &app.pinned else {
        set_class("compare", "hidden", true);
        return;
    };
    set_class("compare", "hidden", false);
    let a = pb.verdict();
    let c = &app.v;
    let row = |label: &str, x: String, y: String, cls: (&str, &str)| {
        let chg = if x != y { " chg" } else { "" };
        format!("<tr><th>{label}</th><td class=\"{}\">{x}</td><td class=\"{}{chg}\">{y}</td></tr>", cls.0, cls.1)
    };
    let tr_txt = |v: &Verdict| format!("{} / {}", fmt_t(v.report.tr_ns), fmt_t(v.report.tr_max_ns));
    let html = format!(
        "<thead><tr><th></th><th>A</th><th>{}</th></tr></thead><tbody>{}{}{}{}</tbody>",
        tr(en, "지금", "Now"),
        row(tr(en, "판정", "Verdict"), a.status.label(en).into(), c.status.label(en).into(),
            (&format!("st-{}", st_class(a.status)), &format!("st-{}", st_class(c.status)))),
        row(tr(en, "풀업 합성", "Pull-ups combined"), fmt_r(a.report.rp_eff), fmt_r(c.report.rp_eff), ("", "")),
        row(tr(en, "상승 시간 / 한계", "Rise time / limit"), tr_txt(&a), tr_txt(c), ("", "")),
        row(tr(en, "버스 용량", "Bus capacitance"), format!("{} pF", sig(a.report.cb_pf)), format!("{} pF", sig(c.report.cb_pf)), ("", "")),
    );
    if let Some(t) = el("cmp") {
        t.set_inner_html(&html);
    }
    set_text(
        "cmp-note",
        if *pb == app.board {
            tr(en, "아직 A와 같아요 — 부품이나 값을 바꿔 보세요.", "Same as A so far — change a part or a value.")
        } else {
            tr(en, "색칠된 칸이 바뀐 값이에요.", "Highlighted cells changed.")
        },
    );
}

fn chip(en: bool, l: Level) -> String {
    let (c, t) = match l {
        Level::Pass => ("pass", tr(en, "통과", "pass")),
        Level::Warn => ("warn", tr(en, "주의", "warn")),
        Level::Fail => ("fail", tr(en, "실패", "fail")),
    };
    format!("<span class=\"chip {c}\">{t}</span>")
}

fn clock_level(d: &Design, r: &Report) -> Level {
    if !r.reaches_vih {
        Level::Fail
    } else if r.f_max_hz < d.speed.clock_hz() {
        Level::Warn
    } else {
        Level::Pass
    }
}

/// (label, value, level) rows shared by the result list and the CSV export.
fn rows(app: &App) -> Vec<(String, String, Option<Level>)> {
    let en = app.en;
    let (d, r) = (app.v.design, app.v.report);
    let mut out = vec![
        (tr(en, "판정", "Verdict").into(), format!("{} — {}", app.v.status.label(en), app.v.text(en).0), None),
        (tr(en, "총 버스 용량 Cb", "Total bus capacitance Cb").into(),
         format!("{} pF / {} {} pF", sig(r.cb_pf), tr(en, "한계", "limit"), CB_FAIL_PF), Some(grade(r.cb_pf, CB_FAIL_PF))),
        (tr(en, "Rp_min (싱크 전류)", "Rp_min (sink current)").into(), fmt_r(r.rp_min), None),
        (tr(en, "Rp_max (상승 시간)", "Rp_max (rise time)").into(), fmt_r(r.rp_max), None),
        (tr(en, "가능한 범위", "Feasible window").into(),
         if r.feasible { format!("{} – {}", fmt_r(r.rp_min), fmt_r(r.rp_max)) } else { tr(en, "없음", "none").into() },
         Some(if r.feasible { Level::Pass } else { Level::Fail })),
        (tr(en, "추천 E12 값 (합성)", "Recommended E12 (combined)").into(), r.e12.map(fmt_r).unwrap_or_else(|| "—".into()), None),
        (tr(en, "합성 풀업 Rp_eff", "Effective pull-up Rp_eff").into(), fmt_r(r.rp_eff), None),
        (tr(en, "상승 시간 tr", "Rise time tr").into(),
         format!("{} / max {} ({} {}%)", fmt_t(r.tr_ns), fmt_t(r.tr_max_ns), tr(en, "여유", "margin"), sig(r.margin_pct)),
         r.tr_ns.is_finite().then_some(r.tr_level).or(Some(Level::Fail))),
        (tr(en, "LOW 시 싱크 전류", "Sink current when low").into(),
         format!("{} mA / IOL {} mA", sig(r.sink_ma), sig(d.iol_ma)), Some(r.sink_level)),
        (tr(en, "추정 최대 클럭", "Estimated max clock").into(),
         format!("{} ({} {})", fmt_f(r.f_max_hz), tr(en, "모드", "mode"), fmt_f(d.speed.clock_hz())), Some(clock_level(&d, &r))),
        (tr(en, "정적 전력 (LOW 유지, 라인당)", "Static power held low, per line").into(), format!("{} mW", sig(r.power_mw)), None),
    ];
    if let Some(pb) = &app.pinned {
        let p = pb.verdict().report;
        out.push(("Δ tr (A → now)".into(), format!("{:+.1} ns", r.tr_ns - p.tr_ns), None));
        out.push(("Δ f_max (A → now)".into(), format!("{:+.1} kHz", (r.f_max_hz - p.f_max_hz) / 1e3), None));
    }
    out
}

fn render_details(app: &App) {
    let en = app.en;
    let (d, r) = (app.v.design, app.v.report);
    set_val("vdd", &fmt_in(d.vdd));
    set_val("devices", &fmt_in(d.devices));
    set_val("pin_pf", &fmt_in(d.devices * d.pin_pf));
    set_val("length_cm", &fmt_in(d.length_cm));
    set_val("rp", &fmt_in(r.rp_eff / 1e3));
    set_text("hud-cb", &format!("{} pF", sig(r.cb_pf)));
    set_text("hud-tr", &fmt_t(r.tr_ns));
    set_text("hud-rp", &fmt_r(r.rp_eff));
    set_text("hud-f", &fmt_f(r.f_max_hz));
    let open = el("details").map(|e| e.has_attribute("open")).unwrap_or(false);
    if !open {
        return;
    }
    let html: String = rows(app)
        .iter()
        .map(|(k, v, l)| format!("<li><span>{k}</span><b>{v}</b>{}</li>", l.map(|l| chip(en, l)).unwrap_or_default()))
        .collect();
    if let Some(e) = el("results") {
        e.set_inner_html(&html);
    }
    draw_wave(app, &d, &r);
    draw_plot(app, &d, &r);
}

// ---------- canvas helpers ----------

fn ctx_for(id: &str) -> Option<(Ctx, f64, f64)> {
    let c: HtmlCanvasElement = el(id)?.dyn_into().ok()?;
    let (cw, ch) = (c.client_width(), c.client_height());
    if cw <= 0 || ch <= 0 {
        return None;
    }
    let dpr = web_sys::window()?.device_pixel_ratio().max(1.0);
    let (pw, ph) = ((cw as f64 * dpr).round() as u32, (ch as f64 * dpr).round() as u32);
    if c.width() != pw || c.height() != ph {
        c.set_width(pw);
        c.set_height(ph);
    }
    let ctx: Ctx = c.get_context("2d").ok()??.dyn_into().ok()?;
    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    ctx.clear_rect(0.0, 0.0, cw as f64, ch as f64);
    ctx.set_global_alpha(1.0);
    ctx.set_text_align("left");
    ctx.set_font("11px system-ui, sans-serif");
    dash(&ctx, false);
    Some((ctx, cw as f64, ch as f64))
}

fn dash(ctx: &Ctx, on: bool) {
    let a = js_sys::Array::new();
    if on {
        a.push(&4.0.into());
        a.push(&4.0.into());
    }
    let _ = ctx.set_line_dash(&a);
}

fn line(ctx: &Ctx, pts: impl Iterator<Item = (f64, f64)>, color: &str, width: f64) {
    ctx.begin_path();
    for (i, (x, y)) in pts.enumerate() {
        if i == 0 { ctx.move_to(x, y) } else { ctx.line_to(x, y) }
    }
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(width);
    ctx.stroke();
}

fn seg(ctx: &Ctx, a: (f64, f64), b: (f64, f64), color: &str, width: f64) {
    line(ctx, [a, b].into_iter(), color, width);
}

fn dot(ctx: &Ctx, x: f64, y: f64, r: f64, color: &str) {
    ctx.begin_path();
    let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
    ctx.set_fill_style_str(color);
    ctx.fill();
}

fn text(ctx: &Ctx, s: &str, x: f64, y: f64, font: &str, color: &str, align: &str) {
    ctx.set_font(font);
    ctx.set_fill_style_str(color);
    ctx.set_text_align(align);
    let _ = ctx.fill_text(s, x, y);
}

fn rect(ctx: &Ctx, x: f64, y: f64, w: f64, h: f64, fill: &str) {
    ctx.set_fill_style_str(fill);
    ctx.fill_rect(x, y, w, h);
}

fn outline(ctx: &Ctx, (x, y, w, h): (f64, f64, f64, f64), color: &str, width: f64, dashed: bool) {
    dash(ctx, dashed);
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(width);
    ctx.stroke_rect(x, y, w, h);
    dash(ctx, false);
}

// ---------- scope strip ----------

fn draw_scope(app: &App) {
    let en = app.en;
    let (d, r) = (app.v.design, app.v.report);
    let p = 1e9 / d.speed.clock_hz();
    let missed = !r.reaches_vih;
    let slow = d.speed.clock_hz() / SCOPE_RATE;
    let slow_txt = if en { format!("≈ {:.0}k× slower", slow / 1e3) } else { format!("약 {:.0}만 배 느리게", slow / 1e4) };
    let cap = format!(
        "SCL · {} · {} — {}",
        fmt_f(d.speed.clock_hz()),
        slow_txt,
        if missed {
            tr(en, "빨간 칸: HIGH(VIH)에 닿기 전에 떨어진 비트", "red: the bit fell before reaching HIGH (VIH)")
        } else if r.tr_level == Level::Warn {
            tr(en, "올라가긴 하지만 아슬아슬해요", "it rises, but only just")
        } else {
            tr(en, "모든 비트가 HIGH(VIH)에 닿아요", "every bit reaches HIGH (VIH)")
        }
    );
    set_text("scope-cap", &cap);
    let Some((ctx, w, h)) = ctx_for("scope") else { return };
    let np = 6.0;
    let t0 = if app.motion { (app.anim_t * SCOPE_RATE).rem_euclid(1e6) * p } else { 0.0 };
    let (l, rr, top, bot) = (38.0, 8.0, 10.0, 16.0);
    let v_top = d.vdd * 1.15;
    let span = np * p;
    let x = |t: f64| l + t / span * (w - l - rr);
    let y = |v: f64| top + (1.0 - v / v_top) * (h - top - bot);
    ctx.set_fill_style_str(C_TEXT);
    for v in [0.0, d.vdd] {
        seg(&ctx, (l, y(v)), (w - rr, y(v)), C_GRID, 1.0);
        let _ = ctx.fill_text(&format!("{v:.1} V"), 2.0, y(v) + 4.0);
    }
    // Missed-bit highlights, flashing.
    let first = (t0 / p).floor();
    if missed {
        let alpha = if app.motion { 0.14 + 0.16 * (app.anim_t * 5.0).sin().abs() } else { 0.22 };
        ctx.set_fill_style_str(&format!("rgba(255,107,107,{alpha:.3})"));
        for k in 0..=(np as i32 + 1) {
            let hs = (first + k as f64) * p + p / 2.0 - t0;
            let (a, b) = (hs.max(0.0), (hs + p / 2.0).min(span));
            if b > a {
                ctx.fill_rect(x(a), top, x(b) - x(a), h - top - bot);
            }
        }
    }
    dash(&ctx, true);
    seg(&ctx, (l, y(0.7 * d.vdd)), (w - rr, y(0.7 * d.vdd)), C_WARN, 1.0);
    dash(&ctx, false);
    text(&ctx, "VIH", w - rr - 4.0, y(0.7 * d.vdd) - 4.0, "11px system-ui, sans-serif", C_WARN, "right");
    let n = (w - l - rr).max(10.0) as usize;
    let sample = |i: usize| {
        let tt = span * i as f64 / n as f64;
        let tm = (t0 + tt).rem_euclid(p);
        (tt, tm)
    };
    line(&ctx, (0..=n).map(|i| {
        let (tt, tm) = sample(i);
        (x(tt), y(if tm < p / 2.0 { d.vol } else { d.vdd }))
    }), "rgba(154,167,189,0.25)", 1.2);
    let color = if missed { C_BAD } else if r.tr_level == Level::Warn { C_WARN } else { C_ACC };
    line(&ctx, (0..=n).map(|i| {
        let (tt, tm) = sample(i);
        let v = if tm < p / 2.0 { wave_v(&d, p + tm) } else { wave_v(&d, tm) };
        (x(tt), y(if v.is_finite() { v } else { d.vol }))
    }), color, 2.2);
}

// ---------- Details plots ----------

const M: (f64, f64, f64, f64) = (46.0, 14.0, 16.0, 26.0); // left, right, top, bottom

fn draw_wave(app: &App, d: &Design, r: &Report) {
    let Some((ctx, w, h)) = ctx_for("wave") else { return };
    let p = 1e9 / d.speed.clock_hz();
    let t_end = 1.25 * p;
    let v_top = d.vdd * 1.12;
    let x = |t: f64| M.0 + t / t_end * (w - M.0 - M.1);
    let y = |v: f64| M.2 + (1.0 - v / v_top) * (h - M.2 - M.3);
    ctx.set_fill_style_str(C_TEXT);
    for i in 0..=4 {
        let v = d.vdd * i as f64 / 4.0;
        seg(&ctx, (M.0, y(v)), (w - M.1, y(v)), C_GRID, 1.0);
        let _ = ctx.fill_text(&format!("{v:.1} V"), 4.0, y(v) + 4.0);
    }
    for i in 0..=5 {
        let t = t_end * i as f64 / 5.0;
        let _ = ctx.fill_text(&fmt_t(t), (x(t) - 14.0).min(w - 50.0), h - 8.0);
    }
    // Shaded tr window: from 0.3·VDD to 0.7·VDD on the rise.
    let tau = (r.rp_eff * r.cb_pf * 1e-3).max(1e-6);
    let a = d.vdd - d.vol;
    let t30 = (p / 2.0 + tau * (a / (0.7 * d.vdd)).ln().max(0.0)).min(p);
    let t70 = (p / 2.0 + tau * (a / (0.3 * d.vdd)).ln().max(0.0)).min(p);
    if t30.is_finite() && t70.is_finite() {
        ctx.set_fill_style_str(if r.reaches_vih { "rgba(79,209,197,0.13)" } else { "rgba(255,107,107,0.16)" });
        ctx.fill_rect(x(t30), M.2, (x(t70) - x(t30)).max(0.0), h - M.2 - M.3);
    }
    let sq = [(0.0, d.vol), (p / 2.0, d.vol), (p / 2.0, d.vdd), (p, d.vdd), (p, d.vol), (t_end, d.vol)];
    line(&ctx, sq.iter().map(|(t, v)| (x(*t), y(*v))), "rgba(154,167,189,0.28)", 1.5);
    dash(&ctx, true);
    for (k, lbl) in [(0.7, "VIH"), (0.3, "VIL")] {
        seg(&ctx, (M.0, y(k * d.vdd)), (w - M.1, y(k * d.vdd)), C_WARN, 1.0);
        ctx.set_fill_style_str(C_WARN);
        let _ = ctx.fill_text(lbl, w - M.1 - 26.0, y(k * d.vdd) - 4.0);
    }
    dash(&ctx, false);
    let n = 480;
    let samples = |dd: Design| (0..=n).map(move |i| t_end * i as f64 / n as f64).map(move |t| (t, wave_v(&dd, t)));
    if let Some(pb) = &app.pinned {
        line(&ctx, samples(pb.design()).map(|(t, v)| (x(t), y(v))), C_GHOST, 1.5);
    }
    line(&ctx, samples(*d).map(|(t, v)| (x(t), y(v))), if r.reaches_vih { C_ACC } else { C_BAD }, 2.2);
    if let Some(f) = app.wave_cursor {
        let t = f * t_end;
        let v = wave_v(d, t);
        seg(&ctx, (x(t), M.2), (x(t), h - M.3), "rgba(255,255,255,0.35)", 1.0);
        dot(&ctx, x(t), y(v), 4.0, "#fff");
        tip("wave", x(t), y(v), &format!("t = {} · V = {:.2} V", fmt_t(t), v));
    }
}

fn plot_xy(w: f64, h: f64) -> (impl Fn(f64) -> f64, impl Fn(f64) -> f64) {
    let px = move |cb: f64| M.0 + (cb.clamp(CB_X.0, CB_X.1) - CB_X.0) / (CB_X.1 - CB_X.0) * (w - M.0 - M.1);
    let (l0, l1) = (RP_Y.0.log10(), RP_Y.1.log10());
    let py = move |rp: f64| M.2 + (1.0 - (rp.clamp(RP_Y.0, RP_Y.1).log10() - l0) / (l1 - l0)) * (h - M.2 - M.3);
    (px, py)
}

fn plot_inv(w: f64, h: f64, mx: f64, my: f64) -> (f64, f64) {
    let fx = ((mx - M.0) / (w - M.0 - M.1).max(1.0)).clamp(0.0, 1.0);
    let fy = ((my - M.2) / (h - M.2 - M.3).max(1.0)).clamp(0.0, 1.0);
    let (l0, l1) = (RP_Y.0.log10(), RP_Y.1.log10());
    (CB_X.0 + fx * (CB_X.1 - CB_X.0), 10f64.powf(l1 - fy * (l1 - l0)))
}

fn draw_plot(app: &App, d: &Design, r: &Report) {
    let Some((ctx, w, h)) = ctx_for("plot") else { return };
    let (px, py) = plot_xy(w, h);
    ctx.set_fill_style_str(C_TEXT);
    for rp in [100.0, 1e3, 1e4, 1e5] {
        seg(&ctx, (M.0, py(rp)), (w - M.1, py(rp)), C_GRID, 1.0);
        let _ = ctx.fill_text(&fmt_r(rp), 2.0, py(rp) + 4.0);
    }
    for cb in [100.0, 200.0, 300.0, 400.0, 500.0, 600.0] {
        let _ = ctx.fill_text(&format!("{cb} pF"), (px(cb) - 16.0).min(w - 44.0), h - 8.0);
    }
    let n = 120;
    let xs: Vec<f64> = (0..=n).map(|i| CB_X.0 + (CB_X.1 - CB_X.0) * i as f64 / n as f64).collect();
    let tmax = d.speed.tr_max_ns();
    ctx.begin_path();
    ctx.move_to(px(xs[0]), py(r.rp_min));
    for cb in &xs {
        ctx.line_to(px(*cb), py(rp_max_ohm(tmax, *cb).max(r.rp_min)));
    }
    ctx.line_to(px(CB_X.1), py(r.rp_min));
    ctx.close_path();
    ctx.set_fill_style_str("rgba(94,227,143,0.14)");
    ctx.fill();
    line(&ctx, xs.iter().map(|cb| (px(*cb), py(rp_max_ohm(tmax, *cb)))), C_ACC, 1.8);
    seg(&ctx, (M.0, py(r.rp_min)), (w - M.1, py(r.rp_min)), C_WARN, 1.5);
    dash(&ctx, true);
    seg(&ctx, (px(CB_FAIL_PF), M.2), (px(CB_FAIL_PF), h - M.3), C_BAD, 1.0);
    dash(&ctx, false);
    ctx.set_fill_style_str(C_ACC);
    let _ = ctx.fill_text("Rp_max", px(60.0), py(rp_max_ohm(tmax, 60.0)) - 6.0);
    ctx.set_fill_style_str(C_WARN);
    let _ = ctx.fill_text("Rp_min", w - M.1 - 44.0, py(r.rp_min) - 5.0);
    if let Some(pb) = &app.pinned {
        let p = pb.verdict().report;
        dot(&ctx, px(p.cb_pf), py(p.rp_eff), 6.0, C_GHOST);
    }
    let ok = app.v.status != Status::Fails;
    dot(&ctx, px(r.cb_pf), py(r.rp_eff), 8.0, "#fff");
    dot(&ctx, px(r.cb_pf), py(r.rp_eff), 6.0, if ok { C_OK } else { C_BAD });
    if let Some((mx, my)) = app.plot_hover {
        let (cb, rp) = plot_inv(w, h, mx, my);
        tip("plot", mx, my, &format!("Cb = {} pF · Rp = {}", sig(cb), fmt_r(rp)));
    } else if el("plot").map(|e| Some(e) == doc().active_element()).unwrap_or(false) {
        tip("plot", px(r.cb_pf), py(r.rp_eff), &format!("Cb = {} pF · Rp = {}", sig(r.cb_pf), fmt_r(r.rp_eff)));
    }
}

fn tip(canvas: &str, x: f64, y: f64, text: &str) {
    let (Some(t), Some(c)) = (el("tip"), el(canvas)) else { return };
    let (Ok(t), Ok(c)) = (t.dyn_into::<HtmlElement>(), c.dyn_into::<HtmlElement>()) else { return };
    t.set_text_content(Some(text));
    let s = t.style();
    let (ox, oy) = offset_in_hero(&c);
    let left = (ox + x + 12.0).min(ox + c.offset_width() as f64 - 190.0).max(0.0);
    let _ = s.set_property("left", &format!("{left}px"));
    let _ = s.set_property("top", &format!("{}px", oy + y - 34.0));
    let _ = s.set_property("opacity", "1");
}

/// Canvas position relative to the `.hero` section that holds the tooltip.
fn offset_in_hero(c: &HtmlElement) -> (f64, f64) {
    let (Some(hero), r) = (doc().query_selector(".hero").ok().flatten(), c.get_bounding_client_rect()) else { return (0.0, 0.0) };
    let hr = hero.get_bounding_client_rect();
    (r.left() - hr.left(), r.top() - hr.top())
}

fn hide_tip() {
    if let Some(t) = el("tip").and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
        let _ = t.style().set_property("opacity", "0");
    }
}

// ---------- export ----------

fn export_csv() -> Option<()> {
    let csv = APP.with(|a| {
        let app = a.borrow();
        let mut t = vec![vec!["item".to_string(), "value".into(), "status".into()]];
        if let Some(m) = app.board.mcu {
            t.push(vec!["MCU".into(), m.kind.name(true).into(), String::new()]);
        }
        for p in &app.board.parts {
            let detail = match p.kind {
                Kind::Wire => format!("{} cm", fmt_in(p.len_cm)),
                _ => p.pullup_ohm().map(fmt_ohm).unwrap_or_else(|| "no pull-up".into()),
            };
            t.push(vec![format!("part: {}", p.kind.name(true)), detail, String::new()]);
        }
        for (k, v, l) in rows(&app) {
            let s = match l { Some(Level::Pass) => "pass", Some(Level::Warn) => "warn", Some(Level::Fail) => "fail", None => "" };
            t.push(vec![k, v, s.into()]);
        }
        to_csv(&t)
    });
    let parts = js_sys::Array::of1(&JsValue::from_str(&format!("\u{feff}{csv}")));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/csv;charset=utf-8");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).ok()?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).ok()?;
    let a: web_sys::HtmlAnchorElement = doc().create_element("a").ok()?.dyn_into().ok()?;
    a.set_href(&url);
    a.set_download("i2c-pull-board.csv");
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Some(())
}

// ---------- events ----------

fn local_xy(e: &MouseEvent, id: &str) -> (f64, f64) {
    let Some(t) = el(id) else { return (0.0, 0.0) };
    let r = t.get_bounding_client_rect();
    (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())
}

fn over(e: &MouseEvent, id: &str) -> bool {
    let Some(t) = el(id) else { return false };
    let r = t.get_bounding_client_rect();
    let (x, y) = (e.client_x() as f64, e.client_y() as f64);
    x >= r.left() && x <= r.right() && y >= r.top() && y <= r.bottom()
}

fn ghost(show: Option<(&str, f64, f64)>) {
    let Some(g) = el("ghost").and_then(|g| g.dyn_into::<HtmlElement>().ok()) else { return };
    let s = g.style();
    match show {
        Some((label, x, y)) => {
            g.set_text_content(Some(label));
            let _ = s.set_property("display", "block");
            let _ = s.set_property("left", &format!("{x}px"));
            let _ = s.set_property("top", &format!("{y}px"));
        }
        None => {
            let _ = s.set_property("display", "none");
        }
    }
}

fn target_el(e: &Event) -> Option<Element> {
    e.target().and_then(|t| t.dyn_into::<Element>().ok())
}

fn wire_palette() -> Option<()> {
    let body = el("palette-body")?;
    body.set_inner_html(&palette_html());
    let kind_of = |e: &Event| target_el(e).and_then(|t| t.closest("[data-kind]").ok().flatten()).and_then(|b| b.get_attribute("data-kind")).and_then(|k| Kind::from_id(&k));
    on(&body, "pointerdown", move |e: PointerEvent| {
        let Some(kind) = kind_of(&e) else { return };
        if e.button() != 0 {
            return;
        }
        with(|a| {
            a.tapped = false;
            a.drag = Some(Drag { kind, x0: e.client_x() as f64, y0: e.client_y() as f64, active: false, slot: None });
        });
    });
    on(&body, "click", move |e: MouseEvent| {
        let Some(kind) = kind_of(&e) else { return };
        // Pointer taps were already handled on pointerup; this path serves the keyboard.
        if !with(|a| std::mem::replace(&mut a.tapped, false)) {
            add_kind(kind, None);
        }
    });
    let win = web_sys::window()?;
    on(&win, "pointermove", |e: PointerEvent| {
        let Some(mut d) = with(|a| a.drag) else { return };
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        if !d.active && (x - d.x0).hypot(y - d.y0) > 8.0 {
            d.active = true;
        }
        if !d.active {
            return;
        }
        let en = with(|a| a.en);
        ghost(Some((d.kind.name(en), x, y)));
        let id = board_id();
        d.slot = over(&e, id).then(|| {
            if id == "board3d" {
                with(|a| view3d::drop_at(e.client_x() as f64, e.client_y() as f64, &a.board))
            } else {
                let (lx, _) = local_xy(&e, id);
                with(|a| drop_slot(&a.board, lx / board_scale()))
            }
        });
        with(|a| a.drag = Some(d));
        render();
    });
    on(&win, "pointerup", |e: PointerEvent| {
        let Some(d) = with(|a| a.drag.take()) else { return };
        ghost(None);
        if d.active {
            if over(&e, board_id()) {
                add_kind(d.kind, d.slot);
            } else {
                render();
            }
        } else {
            with(|a| a.tapped = true);
            add_kind(d.kind, None);
        }
    });
    on(&win, "pointercancel", |_: Event| {
        with(|a| a.drag = None);
        ghost(None);
        render();
    });
    Some(())
}

/// Keyboard on either board view: ←/→ select a part, Delete / Backspace / Enter remove it.
fn board_keys(e: KeyboardEvent) {
    let key = e.key();
    let hits: Vec<Hit> = with(|a| a.board.mcu.iter().map(|_| Hit::Mcu).chain((0..a.board.parts.len()).map(Hit::Part)).collect());
    if hits.is_empty() {
        return;
    }
    let cur = with(|a| a.sel).and_then(|s| hits.iter().position(|h| *h == s));
    match key.as_str() {
        "ArrowRight" | "ArrowLeft" => {
            let n = hits.len() as i32;
            let step = if key == "ArrowRight" { 1 } else { -1 };
            let i = cur.map(|c| (c as i32 + step).rem_euclid(n)).unwrap_or(0) as usize;
            with(|a| a.sel = Some(hits[i]));
            render();
        }
        "Delete" | "Backspace" | "Enter" => {
            if let Some(c) = cur {
                remove_hit(hits[c]);
            }
        }
        _ => return,
    }
    e.prevent_default();
}

fn board_id() -> &'static str {
    if with(|a| a.three) { "board3d" } else { "board" }
}

fn wire_board() -> Option<()> {
    let board = el("board")?;
    on(&board, "pointermove", |e: PointerEvent| {
        let (mx, my) = local_xy(&e, "board");
        let s = board_scale();
        let (h, en, name) = with(|a| {
            let h = hit_test(&a.board, mx / s, my / s);
            a.hover = h;
            let name = match h {
                Some(Hit::Mcu) => a.board.mcu.map(|p| p.kind.name(a.en)),
                Some(Hit::Part(i)) => a.board.parts.get(i).map(|p| p.kind.name(a.en)),
                None => None,
            };
            (h, a.en, name)
        });
        match (h, name) {
            (Some(_), Some(n)) => tip("board", mx, my, &format!("{n} — {}", tr(en, "누르면 빠져요", "click to remove"))),
            _ => hide_tip(),
        }
        render();
    });
    on(&board, "pointerleave", |_: Event| {
        with(|a| a.hover = None);
        hide_tip();
        render();
    });
    on(&board, "click", |e: MouseEvent| {
        if with(|a| a.drag.is_some()) {
            return;
        }
        let (mx, my) = local_xy(&e, "board");
        let s = board_scale();
        if let Some(h) = with(|a| hit_test(&a.board, mx / s, my / s)) {
            remove_hit(h);
        }
    });
    on(&board, "keydown", board_keys);
    on(&el("board3d")?, "keydown", board_keys);
    on(&board, "blur", |_: Event| {
        with(|a| a.sel = None);
        render();
    });
    Some(())
}

fn wire_details() -> Option<()> {
    on(&el("bus-fields")?, "input", |_: Event| read_bus());
    on(&el("speed")?, "change", |_: Event| read_bus());
    let parts = el("parts")?;
    on(&parts, "input", |e: Event| {
        if let Some(t) = target_el(&e) {
            on_part_input(&t);
        }
    });
    on(&parts, "change", |e: Event| {
        if let Some(t) = target_el(&e) {
            if t.get_attribute("data-f").is_some_and(|f| f == "v5" || f == "len") {
                on_part_input(&t);
            }
        }
    });
    on(&parts, "click", |e: Event| {
        let Some(rm) = target_el(&e).and_then(|t| t.closest("[data-rm]").ok().flatten()).and_then(|b| b.get_attribute("data-rm")) else { return };
        if rm == "m" {
            remove_hit(Hit::Mcu);
        } else if let Ok(i) = rm.parse() {
            remove_hit(Hit::Part(i));
        }
    });
    on(&el("details")?, "toggle", |_: Event| render());
    on(&el("csv")?, "click", |_: Event| {
        let _ = export_csv();
    });

    let wave = el("wave")?;
    on(&wave, "pointermove", |e: PointerEvent| {
        let (mx, _) = local_xy(&e, "wave");
        let w = el("wave").map(|c| c.client_width() as f64).unwrap_or(1.0);
        with(|a| a.wave_cursor = Some(((mx - M.0) / (w - M.0 - M.1).max(1.0)).clamp(0.0, 1.0)));
        render();
    });
    on(&wave, "keydown", |e: KeyboardEvent| {
        let step = match e.key().as_str() { "ArrowLeft" => -0.02, "ArrowRight" => 0.02, _ => return };
        e.prevent_default();
        with(|a| a.wave_cursor = Some((a.wave_cursor.unwrap_or(0.5) + step).clamp(0.0, 1.0)));
        render();
    });
    on(&wave, "focus", |_: Event| {
        with(|a| {
            a.wave_cursor.get_or_insert(0.5);
        });
        render();
    });
    for ev in ["pointerleave", "blur"] {
        on(&wave, ev, |_: Event| {
            with(|a| a.wave_cursor = None);
            hide_tip();
            render();
        });
    }
    let plot = el("plot")?;
    on(&plot, "pointermove", |e: PointerEvent| {
        let xy = local_xy(&e, "plot");
        with(|a| a.plot_hover = Some(xy));
        render();
    });
    for ev in ["pointerleave", "blur"] {
        on(&plot, ev, |_: Event| {
            with(|a| a.plot_hover = None);
            hide_tip();
            render();
        });
    }
    on(&plot, "focus", |_: Event| render());
    Some(())
}

fn wire() -> Option<()> {
    wire_palette()?;
    wire_board()?;
    wire_details()?;
    for i in 0..4 {
        on(&el(&format!("preset{i}"))?, "click", move |_: Event| {
            if let Some((_, _, b)) = presets().into_iter().nth(i) {
                apply_board(b);
            }
        });
    }
    on(&el("lang")?, "click", |_: Event| {
        let en = with(|a| {
            a.en = !a.en;
            a.en
        });
        if let Some(h) = doc().document_element() {
            let _ = h.set_attribute("lang", if en { "en" } else { "ko" });
        }
        set_text("lang", if en { "한국어" } else { "English" });
        rebuild_parts();
        read_bus();
    });
    on(&el("pin")?, "click", |_: Event| {
        with(|a| a.pinned = if a.pinned.is_some() { None } else { Some(a.board.clone()) });
        render();
    });
    on(&el("hint-close")?, "click", |_: Event| set_class("hint", "hidden", true));
    on(&el("sheet-toggle")?, "click", |_: Event| {
        set_class("palette", "open", !el("palette").map(|e| e.class_list().contains("open")).unwrap_or(false))
    });
    let win = web_sys::window()?;
    let cb = Closure::<dyn FnMut()>::new(render);
    let _ = win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
    cb.forget();
    let reduce = win.match_media("(prefers-reduced-motion: reduce)").ok().flatten().map(|m| m.matches()).unwrap_or(false);
    with(|a| a.motion = !reduce);
    if !reduce {
        start_animation(&win);
    }
    Some(())
}

/// Scope animation: redraw only the scope strip each frame (slowed-down clock).
fn start_animation(win: &web_sys::Window) {
    type Frame = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;
    let f: Frame = Rc::new(RefCell::new(None));
    let g = f.clone();
    let w = win.clone();
    *g.borrow_mut() = Some(Closure::new(move |ts: f64| {
        APP.with(|a| {
            if let Ok(mut app) = a.try_borrow_mut() {
                app.anim_t = ts / 1000.0;
            }
            if let Ok(app) = a.try_borrow() {
                draw_scope(&app);
            }
        });
        if let Some(cb) = f.borrow().as_ref() {
            let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    let first = g.borrow();
    if let Some(cb) = first.as_ref() {
        let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    let three = view3d::init();
    with(|a| a.three = three);
    if !three {
        set_class("stage3d", "hidden", true);
        set_class("board", "hidden", false);
        set_class("gl-note", "hidden", false);
    }
    if wire().is_none() {
        web_sys::console::error_1(&"I2C Pull: page markup is incomplete".into());
    }
    sync_bus_inputs();
    rebuild_parts();
    update();
}
