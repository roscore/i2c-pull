//! WebGL2 breadboard view: draws `scene` with the `gl` renderer, orbit / pan / zoom camera with
//! inertia (mouse + touch), hover and click selection, palette drop ghost, SDA/SCL pulses, culprit
//! glow and HTML labels. Renders only on change or while something animates.
use super::*;
use crate::gl::{Mesh, Paint, Renderer};
use crate::math::{OrbitCam, M4, V};
use crate::scene::{ViewPreset, TOP};
use web_sys::{HtmlElement, ResizeObserver, WheelEvent};

const ACC: [f32; 4] = [0.31, 0.82, 0.77, 1.0];
const BAD: [f32; 4] = [1.0, 0.25, 0.2, 1.0];
const AMBER: V = [1.0, 0.58, 0.12];
const PULSE_SDA: V = [1.0, 0.93, 0.5];
const PULSE_SCL: V = [0.5, 0.82, 1.0];
const PULSES: usize = 6;

struct Model {
    hit: Hit,
    lit: Mesh,
    glow: Option<Mesh>,
    lo: V,
    hi: V,
    on_board: bool,
}

/// Polyline with cumulative arc length, sampled by distance without allocating.
#[derive(Default)]
struct Path {
    pts: Vec<V>,
    cum: Vec<f32>,
}

impl Path {
    fn new(pts: Vec<V>) -> Path {
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        for i in 0..pts.len() {
            if i > 0 {
                let d = math::sub(pts[i], pts[i - 1]);
                acc += math::dot(d, d).sqrt();
            }
            cum.push(acc);
        }
        Path { pts, cum }
    }
    fn len(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }
    fn at(&self, s: f32) -> V {
        if self.pts.len() < 2 {
            return self.pts.first().copied().unwrap_or([0.0; 3]);
        }
        let i = self.cum.partition_point(|c| *c < s).clamp(1, self.pts.len().max(2) - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let span = (self.cum[i] - self.cum[i - 1]).max(1e-6);
        math::add(a, math::scale(math::sub(b, a), ((s - self.cum[i - 1]) / span).clamp(0.0, 1.0)))
    }
}

#[derive(Clone, Copy)]
struct Press {
    x: f32,
    y: f32,
    button: i16,
    moved: bool,
}

/// What the meshes were built from (the verdict, and so the culprit, follows from the board).
type Key = (Board, Status, bool);
type FrameCb = Closure<dyn FnMut(f64)>;

pub struct View {
    r: Renderer,
    canvas: HtmlCanvasElement,
    cam: OrbitCam,
    touched: bool,
    /// Parts on the board when the camera was last framed.
    n_parts: usize,
    board: Mesh,
    cube: Mesh,
    edges: Mesh,
    ball: Mesh,
    models: Vec<Model>,
    boxes: Vec<(Hit, V, V)>,
    key: Option<Key>,
    hot: Vec<Hit>,
    sda: Path,
    scl: Path,
    stop: bool,
    labels: Vec<(HtmlElement, V)>,
    label_vp: M4,
    size: (u32, u32),
    aspect: f32,
    vp: M4,
    inv: M4,
    ptrs: Vec<(i32, f32, f32)>,
    press: Option<Press>,
    last_tap: (f64, f32, f32),
    last_ts: f64,
    pending: bool,
}

thread_local! {
    static VIEW: RefCell<Option<View>> = const { RefCell::new(None) };
    static RAF: RefCell<Option<FrameCb>> = const { RefCell::new(None) };
}

fn with_view<R>(f: impl FnOnce(&mut View) -> R) -> Option<R> {
    VIEW.with(|v| v.try_borrow_mut().ok()?.as_mut().map(f))
}

fn edge_geo() -> gl::Geo {
    let c = |i: usize| -> V { [(i & 1) as f32 - 0.5, (i >> 1 & 1) as f32 - 0.5, (i >> 2 & 1) as f32 - 0.5] };
    let mut pts = vec![];
    for a in 0..8 {
        for bit in [1, 2, 4] {
            if a & bit == 0 {
                pts.push(c(a));
                pts.push(c(a | bit));
            }
        }
    }
    gl::lines(&pts)
}

fn box_model(lo: V, hi: V, pad: f32) -> M4 {
    let c = math::scale(math::add(lo, hi), 0.5);
    let s: V = std::array::from_fn(|i| hi[i] - lo[i] + 2.0 * pad);
    math::basis([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], s, c)
}

fn ball_model(p: V, r: f32) -> M4 {
    math::basis([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [r; 3], p)
}

/// Create the WebGL2 view on `#board3d`. False when WebGL2 (or the shaders) are unavailable.
pub fn init() -> bool {
    let Some(canvas) = el("board3d").and_then(|c| c.dyn_into::<HtmlCanvasElement>().ok()) else { return false };
    let Some(mut r) = Renderer::new(&canvas) else { return false };
    r.theme = scene::DESK;
    r.fog = [90.0, 300.0];
    r.cell = 5.0;
    r.axes = false;
    r.light = math::norm([-0.3, 1.0, -0.25]);
    let (Some(board), Some(cube), Some(edges), Some(ball)) =
        (r.mesh(&scene::breadboard()), r.mesh(&gl::cube()), r.mesh(&edge_geo()), r.mesh(&gl::sphere(10)))
    else {
        return false;
    };
    let view = View {
        r,
        canvas,
        cam: OrbitCam::new(scene::preset(ViewPreset::Persp, 1.6, 3)),
        n_parts: 3,
        touched: false,
        board,
        cube,
        edges,
        ball,
        models: vec![],
        boxes: vec![],
        key: None,
        hot: vec![],
        sda: Path::default(),
        scl: Path::default(),
        stop: false,
        labels: vec![],
        label_vp: [0.0; 16],
        size: (0, 0),
        aspect: 1.6,
        vp: math::IDENTITY,
        inv: math::IDENTITY,
        ptrs: Vec::with_capacity(4),
        press: None,
        last_tap: (-1e9, 0.0, 0.0),
        last_ts: 0.0,
        pending: false,
    };
    VIEW.with(|v| *v.borrow_mut() = Some(view));
    RAF.with(|f| *f.borrow_mut() = Some(Closure::new(on_frame)));
    wire3d().is_some()
}

/// Schedule one frame (no-op if one is already pending).
pub fn request() {
    if with_view(|v| !std::mem::replace(&mut v.pending, true)).unwrap_or(false) {
        raf();
    }
}

fn raf() {
    RAF.with(|f| {
        if let (Some(cb), Some(w)) = (f.borrow().as_ref(), web_sys::window()) {
            let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    });
}

fn on_frame(ts: f64) {
    let again = APP.with(|a| {
        let Ok(app) = a.try_borrow() else { return None };
        with_view(|v| {
            v.pending = false;
            v.frame(&app, ts)
        })
    });
    match again {
        Some(true) => request(),
        Some(false) => {}
        // App busy (an event handler is running): try again next frame.
        None => raf(),
    }
}

/// Insertion slot under a client position (palette drag), or the end of the board.
pub fn drop_at(cx: f64, cy: f64, b: &Board) -> usize {
    with_view(|v| v.ndc(cx, cy).and_then(|(x, y)| scene::pick_board(&v.inv, x, y)).map(|p| scene::drop_slot(b, p[0])))
        .flatten()
        .unwrap_or(b.parts.len())
}

fn label_text(p: &Part, en: bool) -> (String, String) {
    let k = p.kind;
    let name = view2d::short_name(k, en).to_string();
    let sub = match k {
        Kind::Uno | Kind::Esp32 | Kind::RasPi => format!("{}V", k.logic_v().unwrap_or(3.3)),
        Kind::PullUp => p.pullup_ohm().map(fmt_ohm).unwrap_or_default(),
        Kind::Wire => format!("{} cm", fmt_in(p.len_cm)),
        _ => p.addr.map(|a| format!("0x{a:02X}")).unwrap_or_default(),
    };
    (name, sub)
}

/// Selected-part panel (details + remove button).
pub fn panel(app: &App) {
    let en = app.en;
    let p = match app.sel {
        Some(Hit::Mcu) => app.board.mcu,
        Some(Hit::Part(i)) => app.board.parts.get(i).copied(),
        None => None,
    };
    set_class("sel3d", "hidden", p.is_none());
    let Some(p) = p else { return };
    let mut s = p.kind.name(en).to_string();
    if p.kind == Kind::Wire {
        s += &format!(" · {} cm", fmt_in(p.len_cm));
    } else {
        s += &format!(" · {}", p.pullup_ohm().map(fmt_ohm).unwrap_or_else(|| tr(en, "풀업 없음", "no pull-up").into()));
        if !matches!(p.kind, Kind::PullUp) {
            s += &format!(" · {} pF", fmt_in(p.pin_pf));
        }
        if let Some(a) = p.addr {
            s += &format!(" · 0x{a:02X}");
        }
        if p.kind.is_target() && !p.five_v_ok {
            s += tr(en, " · 3.3V 전용", " · 3.3 V only");
        }
    }
    set_text("sel3d-text", &s);
    set_text("sel3d-rm", tr(en, "빼기", "Remove"));
}

impl View {
    fn ndc(&self, cx: f64, cy: f64) -> Option<(f32, f32)> {
        let r = self.canvas.get_bounding_client_rect();
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return None;
        }
        let x = ((cx - r.left()) / r.width()) * 2.0 - 1.0;
        let y = 1.0 - ((cy - r.top()) / r.height()) * 2.0;
        (x.abs() <= 1.0 && y.abs() <= 1.0).then_some((x as f32, y as f32))
    }

    fn pick(&self, cx: f64, cy: f64) -> Option<Hit> {
        let (x, y) = self.ndc(cx, cy)?;
        scene::pick_part(&self.boxes, &self.inv, x, y)
    }

    fn resize(&mut self) {
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0).clamp(1.0, 2.0);
        let (cw, ch) = (self.canvas.client_width().max(1), self.canvas.client_height().max(1));
        let (pw, ph) = ((cw as f64 * dpr).round() as u32, (ch as f64 * dpr).round() as u32);
        if (pw, ph) != self.size {
            self.canvas.set_width(pw);
            self.canvas.set_height(ph);
            self.size = (pw, ph);
            self.aspect = cw as f32 / ch as f32;
            if !self.touched {
                self.cam = OrbitCam::new(scene::preset(ViewPreset::Persp, self.aspect, self.n_parts));
            }
            self.label_vp = [0.0; 16];
        }
    }

    /// Rebuild meshes and labels when the board, verdict or language changed.
    fn sync(&mut self, app: &App) {
        // Compared in place every frame; cloned only when something changed.
        if self.key.as_ref().is_some_and(|k| k.0 == app.board && k.1 == app.v.status && k.2 == app.en) {
            return;
        }
        let hot = scene::culprits(&app.board, &app.v);
        let key: Key = (app.board.clone(), app.v.status, app.en);
        for m in self.models.drain(..) {
            self.r.free(m.lit);
            if let Some(g) = m.glow {
                self.r.free(g);
            }
        }
        let sc = scene::build(&app.board, &app.v);
        self.boxes = sc.parts.iter().map(|p| (p.hit, p.lo, p.hi)).collect();
        let mut anchors = vec![];
        for p in &sc.parts {
            let Some(lit) = self.r.mesh(&p.lit) else { continue };
            let glow = if p.glow.is_empty() { None } else { self.r.mesh(&p.glow) };
            self.models.push(Model { hit: p.hit, lit, glow, lo: p.lo, hi: p.hi, on_board: p.on_board });
            let part = match p.hit {
                Hit::Mcu => app.board.mcu,
                Hit::Part(i) => app.board.parts.get(i).copied(),
            };
            if let Some(part) = part {
                let a = if part.kind == Kind::Wire { [p.anchor[0], TOP + 0.2, scene::BOARD_Z + 0.4] } else { p.anchor };
                anchors.push((p.hit, label_text(&part, app.en), a));
            }
        }
        self.sda = Path::new(sc.sda);
        self.scl = Path::new(sc.scl);
        self.stop = sc.stop;
        self.hot = hot;
        self.key = Some(key);
        self.build_labels(&anchors);
        // Until the user moves the camera, keep the used part of the board framed.
        self.n_parts = app.board.parts.len();
        if !self.touched {
            self.cam.set_goal(scene::preset(ViewPreset::Persp, self.aspect, self.n_parts));
        }
    }

    fn build_labels(&mut self, anchors: &[(Hit, (String, String), V)]) {
        let Some(c) = el("labels3d") else { return };
        let x1 = scene::SLOT_X0 + scene::SLOT_W * self.n_parts_label() as f32 + 0.6;
        let rails = [("VCC +", "vcc", scene::Z_VCC, 0.0), ("GND −", "gnd", scene::Z_GND, 2.0), ("SDA", "sda", scene::Z_SDA, 0.0), ("SCL", "scl", scene::Z_SCL, 1.6)];
        let mut h = String::new();
        for (hit, (name, sub), _) in anchors {
            let cls = if self.hot.contains(hit) { " bad" } else { "" };
            let sub = if sub.is_empty() { String::new() } else { format!("<small>{sub}</small>") };
            h += &format!("<span class=\"lbl{cls}\">{name}{sub}</span>");
        }
        for (t, cls, _, _) in rails {
            h += &format!("<span class=\"lbl rail {cls}\">{t}</span>");
        }
        c.set_inner_html(&h);
        let kids = c.children();
        let pos = anchors.iter().map(|a| a.2).chain(rails.iter().map(|r| [x1 + r.3, TOP, r.2]));
        self.labels = pos.enumerate().filter_map(|(i, p)| Some((kids.item(i as u32)?.dyn_into::<HtmlElement>().ok()?, p))).collect();
        self.label_vp = [0.0; 16];
    }

    /// Rail labels sit just right of the last placed part (inside the framed area).
    fn n_parts_label(&self) -> usize {
        self.key.as_ref().map(|k| k.0.parts.len()).unwrap_or(0).clamp(2, MAX_PARTS - 1)
    }

    fn place_labels(&mut self) {
        if self.label_vp == self.vp {
            return;
        }
        self.label_vp = self.vp;
        let (cw, ch) = (self.canvas.client_width() as f32, self.canvas.client_height() as f32);
        for (e, p) in &self.labels {
            let s = e.style();
            match math::project(&self.vp, *p).filter(|q| q[0].abs() < 1.1 && q[1].abs() < 1.1 && q[2] < 1.0) {
                Some(q) => {
                    let (x, y) = ((q[0] * 0.5 + 0.5) * cw, (0.5 - q[1] * 0.5) * ch);
                    let _ = s.set_property("transform", &format!("translate({x:.1}px,{y:.1}px) translate(-50%,-100%)"));
                    let _ = s.set_property("visibility", "visible");
                }
                None => {
                    let _ = s.set_property("visibility", "hidden");
                }
            }
        }
    }

    /// Draw one frame; true while something still moves.
    fn frame(&mut self, app: &App, ts: f64) -> bool {
        let dt = ((ts - self.last_ts) / 1000.0).clamp(0.0, 0.1) as f32;
        self.last_ts = ts;
        self.resize();
        self.sync(app);
        let moving = self.cam.step(dt);
        let o = self.cam.cur;
        let (view, proj) = scene::view_proj(&o, self.aspect);
        self.vp = math::mul(&proj, &view);
        self.inv = math::invert(&self.vp).unwrap_or(math::IDENTITY);
        let t = if app.motion { (ts / 1000.0) as f32 } else { 0.0 };
        let r = &self.r;
        r.begin(&view, &proj, o.eye(), [0, 0, self.size.0 as i32, self.size.1 as i32]);
        let id = math::IDENTITY;
        let white = [1.0; 4];
        r.draw(&self.board, &id, Paint::lit(white));
        for m in &self.models {
            let glow = if app.hover == Some(m.hit) && app.drag.is_none() {
                0.22
            } else if app.sel == Some(m.hit) {
                0.1
            } else {
                0.0
            };
            r.draw(&m.lit, &id, Paint { glow, ..Paint::lit(white) });
            if let Some(g) = &m.glow {
                r.draw(g, &id, Paint::flat(white));
            }
        }
        r.shadow(&self.board, &id);
        for m in &self.models {
            if m.on_board {
                r.shadow_at(&m.lit, &id, TOP + 0.006);
            } else {
                r.shadow(&m.lit, &id);
            }
        }
        // Translucent and line overlays last.
        let pulse = if app.motion { 0.5 + 0.5 * (t * 4.0).sin() } else { 0.6 };
        for m in self.models.iter().filter(|m| self.hot.contains(&m.hit)) {
            r.draw(&self.cube, &box_model(m.lo, m.hi, 0.12), Paint::flat([1.0, 0.15, 0.1, 0.08 + 0.16 * pulse]));
            r.draw(&self.edges, &box_model(m.lo, m.hi, 0.12), Paint::flat([1.0, 0.3, 0.25, 0.5 + 0.5 * pulse]));
        }
        if let Some(m) = self.models.iter().find(|m| app.sel == Some(m.hit)) {
            r.draw(&self.edges, &box_model(m.lo, m.hi, 0.08), Paint::flat(ACC));
        }
        self.draw_ghost(app);
        self.draw_pulses(app, t);
        self.place_labels();
        moving || (app.motion && (self.sda.len() > 0.0 || !self.hot.is_empty()))
    }

    fn draw_ghost(&self, app: &App) {
        let Some(Drag { active: true, slot: Some(k), kind, .. }) = app.drag else { return };
        let fill = [ACC[0], ACC[1], ACC[2], 0.28];
        let (lo, hi) = if kind.is_mcu() {
            ([-10.6, 0.0, -2.9], [-1.8, 2.0, 2.9])
        } else if app.board.parts.len() >= MAX_PARTS {
            return;
        } else if k < app.board.parts.len() {
            let x = scene::SLOT_X0 + k as f32 * scene::SLOT_W;
            ([x - 0.08, TOP, -2.6], [x + 0.08, TOP + 2.0, 2.6])
        } else {
            let (a, b) = scene::slot_layout(k + 1)[k];
            ([a, TOP, -2.0], [b, TOP + 1.4, 2.6])
        };
        self.r.draw(&self.cube, &box_model(lo, hi, 0.0), Paint::flat(fill));
        self.r.draw(&self.edges, &box_model(lo, hi, 0.0), Paint::flat(ACC));
    }

    fn draw_pulses(&self, app: &App, t: f32) {
        let unstable = app.v.status == Status::Unstable;
        for (line, path, col) in [(0, &self.sda, PULSE_SDA), (1, &self.scl, PULSE_SCL)] {
            let len = path.len();
            if len <= 0.0 {
                continue;
            }
            for i in 0..PULSES {
                let fi = i as f32;
                let mut tt = t * 0.22;
                if unstable && app.motion {
                    // Stop-and-go: every pulse keeps jumping back a little.
                    tt -= 0.05 * (t * 3.0 + fi * 0.37).fract();
                }
                let u = (fi / PULSES as f32 + tt + line as f32 * 0.07).rem_euclid(1.0);
                let fade = if self.stop { 1.0 - ((u - 0.8) / 0.2).clamp(0.0, 1.0) } else { 1.0 };
                let c = if unstable && i % 3 == 1 { AMBER } else { col };
                let p = path.at(u * len);
                self.r.draw(&self.ball, &ball_model(p, 0.24), Paint::flat([c[0], c[1], c[2], 0.25 * fade]));
                self.r.draw(&self.ball, &ball_model(p, 0.1), Paint::flat([c[0], c[1], c[2], fade.max(0.02)]));
            }
        }
        if self.stop && !self.sda.pts.is_empty() {
            // Red glow where the bus dies.
            let a = if app.motion { 0.25 + 0.2 * (t * 5.0).sin() } else { 0.35 };
            for p in [&self.sda, &self.scl] {
                if let Some(e) = p.pts.last() {
                    self.r.draw(&self.ball, &ball_model(*e, 0.32), Paint::flat([BAD[0], BAD[1], BAD[2], a]));
                }
            }
        }
    }

    fn reset(&mut self, p: ViewPreset) {
        self.cam.set_goal(scene::preset(p, self.aspect, self.n_parts));
    }

    fn cam_basis(&self) -> (V, V) {
        let o = self.cam.cur;
        let f = math::norm(math::sub(o.target, o.eye()));
        let s = math::norm(math::cross(f, [0.0, 1.0, 0.0]));
        (s, math::cross(s, f))
    }

    fn pan_px(&mut self, dx: f32, dy: f32) {
        let ch = self.canvas.client_height().max(1) as f32;
        let k = self.cam.cur.dist * 2.0 * (scene::FOVY / 2.0).tan() / ch;
        let (s, u) = self.cam_basis();
        self.cam.pan(math::add(math::scale(s, -dx * k), math::scale(u, dy * k)));
    }
}

enum After {
    Nothing,
    Render,
    Hover(Option<Hit>),
    Tap(Option<Hit>),
}

fn after(a: After) {
    match a {
        After::Nothing => {}
        After::Render => request(),
        After::Hover(h) => {
            if with(|a| std::mem::replace(&mut a.hover, h)) != h {
                request();
            }
        }
        After::Tap(h) => {
            with(|a| a.sel = h);
            render();
        }
    }
}

fn wire3d() -> Option<()> {
    let canvas = el("board3d")?;
    on(&canvas, "pointerdown", |e: PointerEvent| {
        let (x, y) = (e.client_x() as f32, e.client_y() as f32);
        if let Some(t) = e.target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
            let _ = t.focus();
            let _ = t.set_pointer_capture(e.pointer_id());
        }
        after(with_view(|v| {
            if v.ptrs.len() < 2 {
                v.ptrs.push((e.pointer_id(), x, y));
            }
            v.cam.held = true;
            v.touched = true;
            v.press = match (v.ptrs.len(), v.press) {
                (1, _) => Some(Press { x, y, button: e.button(), moved: false }),
                (_, Some(p)) => Some(Press { moved: true, ..p }),
                _ => None,
            };
            After::Render
        }).unwrap_or(After::Nothing));
    });
    on(&canvas, "pointermove", |e: PointerEvent| {
        let (x, y, id) = (e.client_x() as f32, e.client_y() as f32, e.pointer_id());
        let shift = e.shift_key();
        after(with_view(|v| {
            let Some(i) = v.ptrs.iter().position(|p| p.0 == id) else {
                if e.pointer_type() == "mouse" && v.ptrs.is_empty() {
                    let h = v.pick(x as f64, y as f64);
                    let _ = v.canvas.dyn_ref::<HtmlElement>().map(|c| c.style().set_property("cursor", if h.is_some() { "pointer" } else { "grab" }));
                    return After::Hover(h);
                }
                return After::Nothing;
            };
            let (dx, dy) = (x - v.ptrs[i].1, y - v.ptrs[i].2);
            if v.ptrs.len() == 2 {
                let j = 1 - i;
                let (ox, oy) = (v.ptrs[j].1, v.ptrs[j].2);
                let before = (v.ptrs[i].1 - ox).hypot(v.ptrs[i].2 - oy);
                let now = (x - ox).hypot(y - oy);
                if before > 1.0 && now > 1.0 {
                    v.cam.zoom(before / now);
                }
                v.pan_px(dx / 2.0, dy / 2.0);
            } else if let Some(p) = v.press.as_mut() {
                if (x - p.x).hypot(y - p.y) > 5.0 {
                    p.moved = true;
                }
                if p.button == 2 || p.button == 1 || shift {
                    v.pan_px(dx, dy);
                } else {
                    v.cam.rotate(-dx * 0.006, dy * 0.005);
                }
            }
            v.ptrs[i].1 = x;
            v.ptrs[i].2 = y;
            After::Render
        }).unwrap_or(After::Nothing));
    });
    let up = |e: PointerEvent| {
        let (x, y, id) = (e.client_x() as f32, e.client_y() as f32, e.pointer_id());
        let cancel = e.type_() == "pointercancel";
        after(with_view(|v| {
            v.ptrs.retain(|p| p.0 != id);
            if !v.ptrs.is_empty() {
                return After::Render;
            }
            v.cam.held = false;
            let Some(p) = v.press.take() else { return After::Render };
            if cancel || p.moved || p.button != 0 {
                return After::Render;
            }
            let h = v.pick(x as f64, y as f64);
            let now = e.time_stamp();
            if h.is_none() {
                let (t0, x0, y0) = v.last_tap;
                if now - t0 < 350.0 && (x - x0).hypot(y - y0) < 24.0 {
                    v.reset(ViewPreset::Persp);
                    v.last_tap = (-1e9, 0.0, 0.0);
                } else {
                    v.last_tap = (now, x, y);
                }
            }
            After::Tap(h)
        }).unwrap_or(After::Nothing));
    };
    on(&canvas, "pointerup", up);
    on(&canvas, "pointercancel", up);
    on(&canvas, "pointerleave", |e: PointerEvent| {
        if e.pointer_type() == "mouse" {
            after(After::Hover(None));
        }
    });
    on(&canvas, "wheel", |e: WheelEvent| {
        e.prevent_default();
        let scale = if e.delta_mode() == 1 { 30.0 } else { 1.0 };
        let d = (e.delta_y() * scale).clamp(-300.0, 300.0) as f32;
        after(with_view(|v| {
            v.touched = true;
            v.cam.zoom((d * 0.0015).exp());
            After::Render
        }).unwrap_or(After::Nothing));
    });
    on(&canvas, "contextmenu", |e: Event| e.prevent_default());
    for (id, p) in [("view-persp", ViewPreset::Persp), ("view-top", ViewPreset::Top), ("view-side", ViewPreset::Side)] {
        on(&el(id)?, "click", move |_: Event| {
            with_view(|v| {
                v.touched = true;
                v.reset(p);
            });
            request();
        });
    }
    on(&el("sel3d-rm")?, "click", |_: Event| {
        if let Some(h) = with(|a| a.sel) {
            remove_hit(h);
        }
    });
    let cb = Closure::<dyn FnMut()>::new(request);
    let ro = ResizeObserver::new(cb.as_ref().unchecked_ref()).ok()?;
    ro.observe(&canvas);
    cb.forget();
    std::mem::forget(ro);
    Some(())
}
