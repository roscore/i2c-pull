//! Browser front end: reads inputs, runs `ip_core`, draws two Canvas 2D plots.
use ip_core::*;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d as Ctx, Document, Element, Event, HtmlCanvasElement};
use web_sys::{HtmlElement, HtmlInputElement, HtmlSelectElement, KeyboardEvent, PointerEvent};

const PRESETS: [[&str; 12]; 3] = [
    // vdd, speed, devices, pin_pf, length_cm, pf_per_cm, extra_pf, vol, iol, rp, n_extra, r_extra
    ["3.3", "1", "5", "10", "15", "1.0", "10", "0.4", "3", "2.2", "0", "10"],
    ["5", "0", "2", "10", "100", "0.7", "20", "0.4", "3", "4.7", "0", "10"],
    ["3.3", "2", "4", "10", "20", "1.0", "10", "0.4", "20", "2.2", "3", "10"],
];
const CB_X: (f64, f64) = (10.0, 600.0);
const RP_Y: (f64, f64) = (100.0, 100_000.0);
const C_TEXT: &str = "#9aa7bd";
const C_GRID: &str = "rgba(154,167,189,0.14)";
const C_ACC: &str = "#4fd1c5";
const C_WARN: &str = "#f6c453";
const C_BAD: &str = "#ff6b6b";
const C_OK: &str = "#5ee38f";
const C_GHOST: &str = "rgba(190,196,210,0.45)";

#[derive(Default)]
struct App {
    en: bool,
    cur: Option<(Design, Report)>,
    pinned: Option<(Design, Report)>,
    speed: Option<Speed>,
    wave_cursor: Option<f64>,
    plot_hover: Option<(f64, f64)>,
    dragging: bool,
}

thread_local! { static APP: RefCell<App> = RefCell::new(App::default()); }

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
fn speed_sel() -> Option<HtmlSelectElement> {
    el("speed").and_then(|e| e.dyn_into().ok())
}
fn tr<'a>(en: bool, ko: &'a str, eng: &'a str) -> &'a str {
    if en {
        eng
    } else {
        ko
    }
}
fn on<E: JsCast + 'static>(target: &Element, ev: &str, f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(Event)>::new({
        let mut f = f;
        move |e: Event| {
            if let Ok(e) = e.dyn_into::<E>() {
                f(e)
            }
        }
    });
    let _ = target.add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref());
    cb.forget();
}

fn sig(x: f64) -> String {
    if !x.is_finite() {
        return "∞".into();
    }
    let d = if x.abs() >= 100.0 { 0 } else if x.abs() >= 10.0 { 1 } else { 2 };
    format!("{x:.d$}")
}
fn fmt_r(o: f64) -> String {
    if o >= 1e6 {
        format!("{} MΩ", sig(o / 1e6))
    } else if o >= 1e3 {
        format!("{} kΩ", sig(o / 1e3))
    } else {
        format!("{} Ω", sig(o))
    }
}
fn fmt_t(ns: f64) -> String {
    if ns.abs() >= 1000.0 {
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
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Read every field, show inline errors, and return a valid design or None.
fn read_design(en: bool) -> Option<Design> {
    let mut v = [0.0; 11];
    let mut ok = true;
    for (i, f) in Field::ALL.iter().enumerate() {
        let msg = match parse_field(*f, &val(f.id())) {
            Ok(x) => {
                v[i] = x;
                String::new()
            }
            Err(NumErr::Empty) => tr(en, "값을 입력하세요", "Enter a value").into(),
            Err(NumErr::NotNumber) => tr(en, "숫자가 아닙니다", "Not a number").into(),
            Err(NumErr::NotInteger) => tr(en, "정수만 입력", "Whole numbers only").into(),
            Err(NumErr::OutOfRange(a, b)) => {
                format!("{} {}–{}", tr(en, "허용 범위", "Allowed"), fmt_in(a), fmt_in(b))
            }
        };
        ok &= msg.is_empty();
        show_err(f.id(), &msg);
    }
    if !ok {
        return None;
    }
    let speed = Speed::from_index(speed_sel().map(|s| s.selected_index().max(0) as usize).unwrap_or(1));
    let d = Design {
        vdd: v[0],
        speed,
        devices: v[1],
        pin_pf: v[2],
        length_cm: v[3],
        pf_per_cm: v[4],
        extra_pf: v[5],
        vol: v[6],
        iol_ma: v[7],
        rp_kohm: v[8],
        n_extra: v[9],
        r_extra_kohm: v[10],
    };
    match d.check() {
        Ok(()) => Some(d),
        Err(DesignErr::VolNotBelowVdd) => {
            show_err("vol", tr(en, "VOL은 VDD보다 작아야 합니다", "VOL must be below VDD"));
            None
        }
        Err(DesignErr::ZeroCapacitance) => {
            show_err("extra_pf", tr(en, "총 용량이 0 pF입니다", "Total capacitance is 0 pF"));
            None
        }
    }
}

fn show_err(id: &str, msg: &str) {
    set_text(&format!("e-{id}"), msg);
    if let Some(i) = input(id) {
        let _ = i.set_attribute("aria-invalid", if msg.is_empty() { "false" } else { "true" });
    }
}

/// Log-scale slider range for the main pull-up, from the feasible window (data-dependent bounds).
fn slider_range(r: &Report) -> (f64, f64) {
    let lo = (r.rp_min * 0.5).clamp(100.0, 1e6);
    let hi = (r.rp_max * 2.0).clamp(100.0, 1e6).max(lo * 2.0);
    (lo, hi)
}

fn recompute(from_slider: bool) {
    APP.with(|a| {
        let mut app = a.borrow_mut();
        // Swap IOL default with the speed mode unless the user edited it.
        let sp = Speed::from_index(speed_sel().map(|s| s.selected_index().max(0) as usize).unwrap_or(1));
        if let Some(old) = app.speed {
            if old != sp && parse_field(Field::Iol, &val("iol")) == Ok(old.default_iol_ma()) {
                set_val("iol", &fmt_in(sp.default_iol_ma()));
            }
        }
        app.speed = Some(sp);
        if from_slider {
            if let (Some((_, r)), Some(s)) = (app.cur, input("rp_slider")) {
                let (lo, hi) = slider_range(&r);
                let f = s.value_as_number().clamp(0.0, 1000.0) / 1000.0;
                set_val("rp", &fmt_in(lo * (hi / lo).powf(f) / 1e3));
            }
        }
        match read_design(app.en) {
            Some(d) => {
                let r = analyze(&d);
                app.cur = Some((d, r));
                if !from_slider {
                    if let Some(s) = input("rp_slider") {
                        let (lo, hi) = slider_range(&r);
                        let f = ((d.rp_kohm * 1e3).clamp(lo, hi) / lo).ln() / (hi / lo).ln();
                        s.set_value_as_number((f * 1000.0).round());
                    }
                }
                set_class("app", "stale", false);
            }
            None => set_class("app", "stale", true),
        }
    });
    render();
}

fn set_class(id: &str, cls: &str, on: bool) {
    if let Some(e) = el(id) {
        let _ = e.class_list().toggle_with_force(cls, on);
    }
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
    let Some((d, r)) = app.cur else { return vec![] };
    let mut out = vec![
        (tr(en, "총 버스 용량 Cb", "Total bus capacitance Cb").into(),
         format!("{} pF / {} {} pF", sig(r.cb_pf), tr(en, "한계", "limit"), r.cb_limit_pf), Some(r.cb_level)),
        (tr(en, "Rp_min (싱크 전류)", "Rp_min (sink current)").into(), fmt_r(r.rp_min), None),
        (tr(en, "Rp_max (상승 시간)", "Rp_max (rise time)").into(), fmt_r(r.rp_max), None),
        (tr(en, "가능한 범위", "Feasible window").into(),
         if r.feasible { format!("{} – {}", fmt_r(r.rp_min), fmt_r(r.rp_max)) } else { tr(en, "없음", "none").into() },
         Some(if r.feasible { Level::Pass } else { Level::Fail })),
        (tr(en, "추천 E12 값 (실효)", "Recommended E12 (effective)").into(),
         r.e12.map(fmt_r).unwrap_or_else(|| "—".into()), None),
    ];
    if d.n_extra > 0.0 {
        let main = r.e12.and_then(|t| solve_main_rp(t, d.n_extra, d.r_extra_kohm * 1e3));
        out.push((tr(en, "→ 메인 보드 저항", "→ main-board resistor").into(),
                  main.map(fmt_r).unwrap_or_else(|| tr(en, "모듈 풀업만으로 이미 낮음", "module pull-ups alone are already lower").into()), None));
    }
    out.extend([
        (tr(en, "실효 풀업 Rp", "Effective pull-up Rp").into(), fmt_r(r.rp_eff), None),
        (tr(en, "상승 시간 tr", "Rise time tr").into(),
         format!("{} / max {} ({} {}%)", fmt_t(r.tr_ns), fmt_t(r.tr_max_ns), tr(en, "여유", "margin"), sig(r.margin_pct)), Some(r.tr_level)),
        (tr(en, "LOW 시 싱크 전류", "Sink current when low").into(),
         format!("{} mA / IOL {} mA", sig(r.sink_ma), sig(d.iol_ma)), Some(r.sink_level)),
        (tr(en, "추정 최대 클럭", "Estimated max clock").into(),
         format!("{} ({} {})", fmt_f(r.f_max_hz), tr(en, "모드", "mode"), fmt_f(d.speed.clock_hz())), Some(clock_level(&d, &r))),
        (tr(en, "정적 전력 (LOW 유지, 라인당)", "Static power held low, per line").into(), format!("{} mW", sig(r.power_mw)), None),
    ]);
    if let Some((_, p)) = app.pinned {
        out.push(("Δ tr".into(), format!("{:+.1} ns", r.tr_ns - p.tr_ns), None));
        out.push(("Δ f_max".into(), format!("{:+.1} kHz", (r.f_max_hz - p.f_max_hz) / 1e3), None));
    }
    out
}

fn render() {
    APP.with(|a| {
        let app = a.borrow();
        let en = app.en;
        let html: String = rows(&app)
            .iter()
            .map(|(k, v, l)| format!("<li><span>{k}</span><b>{v}</b>{}</li>", l.map(|l| chip(en, l)).unwrap_or_default()))
            .collect();
        if let Some(e) = el("results") {
            e.set_inner_html(&html);
        }
        if let Some((d, r)) = app.cur {
            set_text("hud-cb", &format!("{} pF", sig(r.cb_pf)));
            set_text("hud-tr", &fmt_t(r.tr_ns));
            set_text("hud-rp", &fmt_r(r.rp_eff));
            set_text("hud-f", &fmt_f(r.f_max_hz));
            let verdict = if r.feasible {
                format!("{} {} – {}{}", tr(en, "사용 가능한 풀업:", "Usable pull-up:"), fmt_r(r.rp_min), fmt_r(r.rp_max),
                        r.e12.map(|v| format!(" · {} {}", tr(en, "추천", "pick"), fmt_r(v))).unwrap_or_default())
            } else {
                tr(en, "이 조건에서는 어떤 저항도 맞지 않습니다. 용량을 줄이거나, 속도를 낮추거나, 버스 버퍼/액티브 풀업을 쓰세요.",
                   "No resistor works here: reduce capacitance, lower the speed, or use a bus buffer / active pull-up.").into()
            };
            set_text("verdict", &verdict);
            set_class("verdict", "bad", !r.feasible);
            set_text("cb-total", &format!("Cb = {} pF ({} {} pF)", sig(r.cb_pf), tr(en, "한계", "limit"), r.cb_limit_pf));
            set_text("rp-eff", &format!("{} {}", tr(en, "실효", "effective"), fmt_r(r.rp_eff)));
            draw_wave(&app, &d, &r);
            draw_plot(&app, &d, &r);
        }
        set_text("pin", if app.pinned.is_some() { tr(en, "비교 해제", "Unpin") } else { tr(en, "비교 고정", "Pin to compare") });
    });
}

fn ctx_for(id: &str) -> Option<(Ctx, f64, f64)> {
    let c: HtmlCanvasElement = el(id)?.dyn_into().ok()?;
    let dpr = web_sys::window()?.device_pixel_ratio().max(1.0);
    let (w, h) = (c.client_width().max(1) as f64, c.client_height().max(1) as f64);
    c.set_width((w * dpr) as u32);
    c.set_height((h * dpr) as u32);
    let ctx: Ctx = c.get_context("2d").ok()??.dyn_into().ok()?;
    let _ = ctx.scale(dpr, dpr);
    ctx.clear_rect(0.0, 0.0, w, h);
    ctx.set_font("11px system-ui, sans-serif");
    Some((ctx, w, h))
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
        line(&ctx, [(M.0, y(v)), (w - M.1, y(v))].into_iter(), C_GRID, 1.0);
        let _ = ctx.fill_text(&format!("{v:.1} V"), 4.0, y(v) + 4.0);
    }
    for i in 0..=5 {
        let t = t_end * i as f64 / 5.0;
        let _ = ctx.fill_text(&fmt_t(t), (x(t) - 14.0).min(w - 50.0), h - 8.0);
    }
    // Shaded tr window: from 0.3·VDD to 0.7·VDD on the rise.
    let tau = (r.rp_eff * r.cb_pf * 1e-3).max(1e-6);
    let a = d.vdd - d.vol;
    let t30 = p / 2.0 + tau * (a / (0.7 * d.vdd)).ln().max(0.0);
    let t70 = (p / 2.0 + tau * (a / (0.3 * d.vdd)).ln().max(0.0)).min(p);
    ctx.set_fill_style_str(if r.reaches_vih { "rgba(79,209,197,0.13)" } else { "rgba(255,107,107,0.16)" });
    ctx.fill_rect(x(t30.min(p)), M.2, (x(t70) - x(t30.min(p))).max(0.0), h - M.2 - M.3);
    // Ideal square wave.
    let sq = [(0.0, d.vol), (p / 2.0, d.vol), (p / 2.0, d.vdd), (p, d.vdd), (p, d.vol), (t_end, d.vol)];
    line(&ctx, sq.iter().map(|(t, v)| (x(*t), y(*v))), "rgba(154,167,189,0.28)", 1.5);
    dash(&ctx, true);
    for (k, lbl) in [(0.7, "VIH"), (0.3, "VIL")] {
        line(&ctx, [(M.0, y(k * d.vdd)), (w - M.1, y(k * d.vdd))].into_iter(), C_WARN, 1.0);
        ctx.set_fill_style_str(C_WARN);
        let _ = ctx.fill_text(lbl, w - M.1 - 26.0, y(k * d.vdd) - 4.0);
    }
    dash(&ctx, false);
    let n = 480;
    let samples = |dd: Design| (0..=n).map(move |i| t_end * i as f64 / n as f64).map(move |t| (t, wave_v(&dd, t)));
    if let Some((pd, _)) = app.pinned {
        line(&ctx, samples(pd).map(|(t, v)| (x(t), y(v))), C_GHOST, 1.5);
    }
    line(&ctx, samples(*d).map(|(t, v)| (x(t), y(v))), if r.reaches_vih { C_ACC } else { C_BAD }, 2.2);
    if let Some(f) = app.wave_cursor {
        let t = f * t_end;
        let v = wave_v(d, t);
        line(&ctx, [(x(t), M.2), (x(t), h - M.3)].into_iter(), "rgba(255,255,255,0.35)", 1.0);
        dot(&ctx, x(t), y(v), 4.0, "#fff");
        tip("wave", x(t), y(v), &format!("t = {} · V = {:.2} V", fmt_t(t), v));
    }
}

fn dot(ctx: &Ctx, x: f64, y: f64, r: f64, color: &str) {
    ctx.begin_path();
    let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
    ctx.set_fill_style_str(color);
    ctx.fill();
}

fn plot_xy(w: f64, h: f64) -> (impl Fn(f64) -> f64, impl Fn(f64) -> f64) {
    let px = move |cb: f64| M.0 + (cb - CB_X.0) / (CB_X.1 - CB_X.0) * (w - M.0 - M.1);
    let (l0, l1) = (RP_Y.0.log10(), RP_Y.1.log10());
    let py = move |rp: f64| M.2 + (1.0 - (rp.clamp(RP_Y.0, RP_Y.1).log10() - l0) / (l1 - l0)) * (h - M.2 - M.3);
    (px, py)
}

/// Inverse mapping: canvas px → (Cb pF, Rp Ω), clamped to the plot range.
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
        line(&ctx, [(M.0, py(rp)), (w - M.1, py(rp))].into_iter(), C_GRID, 1.0);
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
    line(&ctx, [(M.0, py(r.rp_min)), (w - M.1, py(r.rp_min))].into_iter(), C_WARN, 1.5);
    dash(&ctx, true);
    line(&ctx, [(px(r.cb_limit_pf), M.2), (px(r.cb_limit_pf), h - M.3)].into_iter(), C_BAD, 1.0);
    dash(&ctx, false);
    ctx.set_fill_style_str(C_ACC);
    let _ = ctx.fill_text("Rp_max", px(60.0), py(rp_max_ohm(tmax, 60.0)) - 6.0);
    ctx.set_fill_style_str(C_WARN);
    let _ = ctx.fill_text("Rp_min", w - M.1 - 44.0, py(r.rp_min) - 5.0);
    let cbx = |c: f64| px(c.clamp(CB_X.0, CB_X.1));
    if let Some((_, p)) = app.pinned {
        dot(&ctx, cbx(p.cb_pf), py(p.rp_eff), 6.0, C_GHOST);
    }
    let ok = r.rp_eff >= r.rp_min && r.rp_eff <= r.rp_max && r.cb_level != Level::Fail;
    dot(&ctx, cbx(r.cb_pf), py(r.rp_eff), 8.0, "#fff");
    dot(&ctx, cbx(r.cb_pf), py(r.rp_eff), 6.0, if ok { C_OK } else { C_BAD });
    if let Some((mx, my)) = app.plot_hover {
        let (cb, rp) = plot_inv(w, h, mx, my);
        tip("plot", mx, my, &format!("Cb = {} pF · Rp = {}", sig(cb), fmt_r(rp)));
    } else if el("plot").map(|e| Some(e) == doc().active_element()).unwrap_or(false) {
        tip("plot", cbx(r.cb_pf), py(r.rp_eff), &format!("Cb = {} pF · Rp = {}", sig(r.cb_pf), fmt_r(r.rp_eff)));
    }
}

fn tip(canvas: &str, x: f64, y: f64, text: &str) {
    let (Some(t), Some(c)) = (el("tip"), el(canvas)) else { return };
    let (Ok(t), Ok(c)) = (t.dyn_into::<HtmlElement>(), c.dyn_into::<HtmlElement>()) else { return };
    t.set_text_content(Some(text));
    let s = t.style();
    let left = (c.offset_left() as f64 + x + 12.0).min(c.offset_left() as f64 + c.offset_width() as f64 - 190.0).max(0.0);
    let _ = s.set_property("left", &format!("{left}px"));
    let _ = s.set_property("top", &format!("{}px", c.offset_top() as f64 + y - 34.0));
    let _ = s.set_property("opacity", "1");
}

fn hide_tip() {
    if let Some(t) = el("tip").and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
        let _ = t.style().set_property("opacity", "0");
    }
}

/// Direct manipulation: put the design dot at (Cb, effective Rp) by editing extra pF and main Rp.
fn move_dot(cb: f64, rp_eff: f64) {
    let d = APP.with(|a| a.borrow().cur.map(|c| c.0));
    let Some(d) = d else { return };
    let (lo, hi) = Field::ExtraPf.range();
    set_val("extra_pf", &fmt_in((cb - d.base_cb_pf()).round().clamp(lo, hi.max(lo))));
    let (rlo, rhi) = Field::Rp.range();
    let main = solve_main_rp(rp_eff, d.n_extra, d.r_extra_kohm * 1e3).map(|r| r / 1e3).unwrap_or(rhi);
    let v = (main.clamp(rlo, rhi.max(rlo)) * 100.0).round() / 100.0;
    set_val("rp", &fmt_in(v));
    recompute(false);
}

fn apply_preset(i: usize) {
    let p = PRESETS[i.min(PRESETS.len() - 1)];
    let ids = ["vdd", "speed", "devices", "pin_pf", "length_cm", "pf_per_cm", "extra_pf", "vol", "iol", "rp", "n_extra", "r_extra"];
    for (id, v) in ids.iter().zip(p.iter()) {
        if *id == "speed" {
            if let Some(s) = speed_sel() {
                s.set_selected_index(v.parse().unwrap_or(1));
            }
        } else {
            set_val(id, v);
        }
    }
    APP.with(|a| a.borrow_mut().speed = None);
    recompute(false);
}

fn export_csv() -> Option<()> {
    let csv = APP.with(|a| {
        let app = a.borrow();
        let mut t = vec![vec!["item".to_string(), "value".into(), "status".into()]];
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
    a.set_download("i2c-pull-results.csv");
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Some(())
}

fn local_xy(e: &PointerEvent) -> (f64, f64) {
    let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return (0.0, 0.0) };
    let r = t.get_bounding_client_rect();
    (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())
}

fn canvas_size(id: &str) -> (f64, f64) {
    el(id).map(|e| (e.client_width() as f64, e.client_height() as f64)).unwrap_or((1.0, 1.0))
}

fn wire() -> Option<()> {
    let settings = el("settings")?;
    on(&settings, "input", |e: Event| {
        let slider = e.target().and_then(|t| t.dyn_into::<Element>().ok()).map(|t| t.id() == "rp_slider").unwrap_or(false);
        recompute(slider);
    });
    for (i, id) in ["preset0", "preset1", "preset2"].iter().enumerate() {
        on(&el(id)?, "click", move |_: Event| apply_preset(i));
    }
    on(&el("lang")?, "click", |_: Event| {
        let en = APP.with(|a| {
            let mut a = a.borrow_mut();
            a.en = !a.en;
            a.en
        });
        if let Some(h) = doc().document_element() {
            let _ = h.set_attribute("lang", if en { "en" } else { "ko" });
        }
        set_text("lang", if en { "한국어" } else { "English" });
        recompute(false);
    });
    on(&el("pin")?, "click", |_: Event| {
        APP.with(|a| {
            let mut a = a.borrow_mut();
            a.pinned = if a.pinned.is_some() { None } else { a.cur };
        });
        render();
    });
    on(&el("csv")?, "click", |_: Event| {
        let _ = export_csv();
    });
    on(&el("hint-close")?, "click", |_: Event| set_class("hint", "hidden", true));
    on(&el("sheet-toggle")?, "click", |_: Event| set_class("settings", "open", !el("settings").map(|e| e.class_list().contains("open")).unwrap_or(false)));

    let wave = el("wave")?;
    on(&wave, "pointermove", |e: PointerEvent| {
        let (mx, _) = local_xy(&e);
        let (w, _) = canvas_size("wave");
        APP.with(|a| a.borrow_mut().wave_cursor = Some(((mx - M.0) / (w - M.0 - M.1).max(1.0)).clamp(0.0, 1.0)));
        render();
    });
    on(&wave, "keydown", |e: KeyboardEvent| {
        let step = match e.key().as_str() { "ArrowLeft" => -0.02, "ArrowRight" => 0.02, _ => return };
        e.prevent_default();
        APP.with(|a| {
            let mut a = a.borrow_mut();
            a.wave_cursor = Some((a.wave_cursor.unwrap_or(0.5) + step).clamp(0.0, 1.0));
        });
        render();
    });
    on(&wave, "focus", |_: Event| {
        APP.with(|a| {
            a.borrow_mut().wave_cursor.get_or_insert(0.5);
        });
        render();
    });
    for ev in ["pointerleave", "blur"] {
        on(&wave, ev, |_: Event| {
            APP.with(|a| a.borrow_mut().wave_cursor = None);
            hide_tip();
            render();
        });
    }

    let plot = el("plot")?;
    on(&plot, "pointerdown", |e: PointerEvent| {
        if let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) {
            let _ = t.set_pointer_capture(e.pointer_id());
        }
        APP.with(|a| a.borrow_mut().dragging = true);
        let (mx, my) = local_xy(&e);
        let (w, h) = canvas_size("plot");
        let (cb, rp) = plot_inv(w, h, mx, my);
        move_dot(cb, rp);
    });
    on(&plot, "pointermove", |e: PointerEvent| {
        let (mx, my) = local_xy(&e);
        let dragging = APP.with(|a| {
            let mut a = a.borrow_mut();
            a.plot_hover = Some((mx, my));
            a.dragging
        });
        if dragging {
            let (w, h) = canvas_size("plot");
            let (cb, rp) = plot_inv(w, h, mx, my);
            move_dot(cb, rp);
        } else {
            render();
        }
    });
    for ev in ["pointerup", "pointercancel"] {
        on(&plot, ev, |_: Event| APP.with(|a| a.borrow_mut().dragging = false));
    }
    for ev in ["pointerleave", "blur"] {
        on(&plot, ev, |_: Event| {
            APP.with(|a| a.borrow_mut().plot_hover = None);
            hide_tip();
            render();
        });
    }
    on(&plot, "focus", |_: Event| render());
    on(&plot, "keydown", |e: KeyboardEvent| {
        let Some((_, r)) = APP.with(|a| a.borrow().cur) else { return };
        let (cb, rp) = match e.key().as_str() {
            "ArrowUp" => (r.cb_pf, r.rp_eff * 1.05),
            "ArrowDown" => (r.cb_pf, r.rp_eff / 1.05),
            "ArrowRight" => (r.cb_pf + 5.0, r.rp_eff),
            "ArrowLeft" => (r.cb_pf - 5.0, r.rp_eff),
            _ => return,
        };
        e.prevent_default();
        move_dot(cb, rp);
    });
    let win = web_sys::window()?;
    let cb = Closure::<dyn FnMut()>::new(render);
    let _ = win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
    cb.forget();
    Some(())
}

#[wasm_bindgen(start)]
pub fn start() {
    if wire().is_none() {
        web_sys::console::error_1(&"I2C Pull: page markup is incomplete".into());
    }
    apply_preset(0);
}
