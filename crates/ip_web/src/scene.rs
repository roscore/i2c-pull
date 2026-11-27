//! Pure 3D scene for the breadboard view (no web-sys, host-testable): where the board, slots and MCU
//! sit, low-poly part models as coloured triangle batches, jumper paths, resistor colour bands, the
//! SDA/SCL pulse paths and camera presets. Units are centimetres; x runs along the board, y is up and
//! z points toward the viewer (power rails at the back, SDA/SCL rails at the front).
use crate::gl::{self, Geo, Theme};
use crate::math::{self, Orbit, M4, V};
use crate::Hit;
use ip_core::*;

/// Breadboard top surface height.
pub const TOP: f32 = 0.85;
/// Hole pitch (0.1 in).
pub const PITCH: f32 = 0.254;
pub const SLOT_X0: f32 = 0.0;
pub const SLOT_W: f32 = 2.1;
pub const BOARD_X: (f32, f32) = (-1.2, SLOT_X0 + SLOT_W * MAX_PARTS as f32 + 0.4);
/// Half depth of the board (z from −BOARD_Z to BOARD_Z).
pub const BOARD_Z: f32 = 3.25;
pub const Z_VCC: f32 = -2.75;
pub const Z_GND: f32 = -2.45;
pub const Z_SDA: f32 = 2.45;
pub const Z_SCL: f32 = 2.75;
/// First terminal row on either side of the centre channel.
pub const ROW0: f32 = 0.381;
/// Where the MCU jumpers enter the rails.
pub const RAIL_IN: f32 = BOARD_X.0 + 0.5;
pub const FOVY: f32 = 0.75;

pub const C_VCC: V = [0.86, 0.13, 0.11];
pub const C_GND: V = [0.07, 0.07, 0.08];
pub const C_SDA: V = [0.96, 0.78, 0.10];
pub const C_SCL: V = [0.14, 0.44, 0.96];
const C_GOLD: V = [0.86, 0.70, 0.30];
const C_SILVER: V = [0.74, 0.76, 0.79];
const C_BLACK: V = [0.07, 0.07, 0.08];
const C_HOLE: V = [0.16, 0.16, 0.18];
const C_RED: V = [0.98, 0.10, 0.08];
const X: V = [1.0, 0.0, 0.0];
const Y: V = [0.0, 1.0, 0.0];
const Z: V = [0.0, 0.0, 1.0];

/// Warm dark desk mat.
pub const DESK: Theme = Theme {
    sky_top: [0.05, 0.05, 0.07],
    horizon: [0.17, 0.14, 0.12],
    checker: [[0.20, 0.17, 0.15], [0.215, 0.185, 0.16]],
    grid: [0.27, 0.23, 0.19],
    amb_sky: [0.44, 0.44, 0.48],
    amb_ground: [0.20, 0.17, 0.15],
    shadow: 0.40,
};

/// Resistor colour code (black … white), plus gold for the tolerance band.
pub const BAND_RGB: [V; 10] = [
    [0.06, 0.06, 0.06],
    [0.45, 0.25, 0.10],
    [0.85, 0.10, 0.10],
    [0.95, 0.45, 0.08],
    [0.95, 0.82, 0.10],
    [0.15, 0.60, 0.22],
    [0.13, 0.33, 0.85],
    [0.50, 0.25, 0.70],
    [0.52, 0.52, 0.52],
    [0.95, 0.95, 0.95],
];

/// 4-band colour code digits (first, second, multiplier exponent) for a value in ohms.
pub fn bands(ohm: f64) -> Option<[usize; 3]> {
    if !ohm.is_finite() || ohm < 10.0 {
        return None;
    }
    let mut e = ohm.log10().floor() as i32 - 1;
    let mut two = (ohm / 10f64.powi(e)).round() as i32;
    if two >= 100 {
        two /= 10;
        e += 1;
    }
    Some([(two / 10).clamp(0, 9) as usize, (two % 10).clamp(0, 9) as usize, e.clamp(0, 9) as usize])
}

/// The part kind the verdict blames (same rule the 2D board uses for its red outline).
pub fn issue_kind(i: Option<&Issue>) -> Option<Kind> {
    match i? {
        Issue::LevelHazard { part } => Some(*part),
        Issue::AddressClash { kind, .. } => Some(*kind),
        Issue::TooStrong { culprit, .. } => *culprit,
        _ => None,
    }
}

/// Every placed part of the blamed kind (the MCU too when it is the culprit).
pub fn culprits(b: &Board, v: &Verdict) -> Vec<Hit> {
    let Some(k) = issue_kind(v.primary()) else { return vec![] };
    let mcu = (b.mcu.map(|m| m.kind) == Some(k)).then_some(Hit::Mcu);
    mcu.into_iter().chain(b.parts.iter().enumerate().filter(|(_, p)| p.kind == k).map(|(i, _)| Hit::Part(i))).collect()
}

pub fn slot_center(i: usize) -> f32 {
    SLOT_X0 + (i as f32 + 0.5) * SLOT_W
}

/// x ranges (left, right) of the first `n` slots.
pub fn slot_layout(n: usize) -> Vec<(f32, f32)> {
    (0..n.min(MAX_PARTS)).map(|i| (SLOT_X0 + i as f32 * SLOT_W + 0.08, SLOT_X0 + (i + 1) as f32 * SLOT_W - 0.08)).collect()
}

/// Insertion index for a drop at board x (between slots, like the 2D board).
pub fn drop_slot(b: &Board, x: f32) -> usize {
    (((x - SLOT_X0) / SLOT_W + 0.5).floor().max(0.0) as usize).min(b.parts.len())
}

/// Loop size for a jumper of `len_cm`: a 50 cm wire hangs in a much bigger arc than a 5 cm one.
pub fn loop_size(len_cm: f64) -> f32 {
    0.5 + 0.13 * len_cm.clamp(0.0, 200.0) as f32
}

/// Jumper loop between two holes `a`, `b`, rising and leaning toward the viewer (+z).
pub fn wire_loop(a: V, b: V, s: f32) -> Vec<V> {
    let lift = [0.0, 0.35, 0.0];
    let mid = math::scale(math::add(a, b), 0.5);
    let mid = math::add(mid, lift);
    let hw = 0.5 * (b[0] - a[0]).abs() + 0.3 * s;
    let mut c = vec![math::add(a, [0.0, -0.1, 0.0]), math::add(a, lift)];
    for k in 1..8 {
        let th = std::f32::consts::PI * (1.0 - k as f32 / 8.0);
        let (sn, cs) = th.sin_cos();
        c.push(math::add(mid, [cs * hw, 0.45 * s * sn, 0.85 * s * sn]));
    }
    c.push(math::add(b, lift));
    c.push(math::add(b, [0.0, -0.1, 0.0]));
    gl::catmull_rom(&c, 6)
}

/// The SDA loop of a jumper-wire part in slot centre `xc` (its length follows `len_cm`).
pub fn sda_loop(xc: f32, len_cm: f64) -> Vec<V> {
    let (a, b) = wire_ends(xc, Z_SDA, 0.55);
    wire_loop(a, b, loop_size(len_cm))
}

fn wire_ends(xc: f32, z: f32, hw: f32) -> (V, V) {
    ([xc - hw, TOP, z], [xc + hw, TOP, z])
}

/// Arched jumper from hole `a` to hole `b` with peak height `h` above the higher end.
fn arc(a: V, b: V, h: f32) -> Vec<V> {
    let top = a[1].max(b[1]) + h;
    let mid = math::scale(math::add(a, b), 0.5);
    let up = |p: V, f: f32| [p[0], p[1] + (top - p[1]) * f, p[2]];
    let lerp = |t: f32| math::add(a, math::scale(math::sub(b, a), t));
    let q1 = lerp(0.2);
    let q3 = lerp(0.8);
    let c = [
        math::add(a, [0.0, -0.1, 0.0]),
        up(a, 0.45),
        [q1[0], top - 0.12 * h, q1[2]],
        [mid[0], top, mid[2]],
        [q3[0], top - 0.12 * h, q3[2]],
        up(b, 0.45),
        math::add(b, [0.0, -0.1, 0.0]),
    ];
    gl::catmull_rom(&c, 6)
}

struct Kit {
    cube: Geo,
    cyl: Geo,
    cap: Geo,
    quad: Geo,
}

impl Kit {
    fn new() -> Kit {
        Kit { cube: gl::cube(), cyl: gl::cylinder(10), cap: gl::capsule(8, 2.3), quad: gl::plane() }
    }
    /// Box centred at `c`.
    fn bx(&self, g: &mut Geo, c: V, s: V, col: V) {
        g.append(&self.cube, &math::basis(X, Y, Z, s, c), col);
    }
    /// Box standing on `base`.
    fn on(&self, g: &mut Geo, base: V, s: V, col: V) {
        self.bx(g, [base[0], base[1] + s[1] / 2.0, base[2]], s, col);
    }
    /// Flat horizontal rectangle centred at `c`.
    fn flat(&self, g: &mut Geo, c: V, sx: f32, sz: f32, col: V) {
        g.append(&self.quad, &math::basis(X, Y, Z, [sx, 1.0, sz], c), col);
    }
    fn cyl_y(&self, g: &mut Geo, base: V, r: f32, h: f32, col: V) {
        g.append(&self.cyl, &math::basis(X, Y, Z, [2.0 * r, h, 2.0 * r], [base[0], base[1] + h / 2.0, base[2]]), col);
    }
    /// Cylinder along z centred at `c`.
    fn cyl_z(&self, g: &mut Geo, c: V, r: f32, len: f32, col: V) {
        g.append(&self.cyl, &math::basis(X, Z, [0.0, -1.0, 0.0], [2.0 * r, len, 2.0 * r], c), col);
    }
    fn wire(&self, g: &mut Geo, path: &[V], r: f32, col: V) {
        g.append(&gl::tube(path, r, 6), &math::IDENTITY, col);
        // Metal tips going into the holes.
        for p in [path.first(), path.last()].into_iter().flatten() {
            self.cyl_y(g, [p[0], p[1] - 0.2, p[2]], r * 0.45, 0.32, C_SILVER);
        }
    }
    /// Male header strip of `n` pins along x starting at `p0` (base of the plastic).
    fn header(&self, g: &mut Geo, p0: V, n: usize, pins_up: bool) {
        let len = n as f32 * PITCH;
        self.on(g, [p0[0] + len / 2.0 - PITCH / 2.0, p0[1], p0[2]], [len, 0.25, 0.25], C_BLACK);
        for i in 0..n {
            let x = p0[0] + i as f32 * PITCH;
            let (y0, h) = if pins_up { (p0[1], 0.85) } else { (p0[1] - 0.6, 0.85) };
            self.on(g, [x, y0, p0[2]], [0.064, h, 0.064], C_GOLD);
        }
    }
    /// Female header block with holes on top (Uno / Pi style), along x.
    fn socket(&self, g: &mut Geo, c: V, n: usize, rows: usize, col: V) {
        let (w, d) = (n as f32 * PITCH, rows as f32 * PITCH);
        self.on(g, c, [w, 0.85, d], col);
        for r in 0..rows {
            for i in 0..n {
                let x = c[0] - w / 2.0 + (i as f32 + 0.5) * PITCH;
                let z = c[2] - d / 2.0 + (r as f32 + 0.5) * PITCH;
                self.flat(g, [x, c[1] + 0.853, z], 0.11, 0.11, C_HOLE);
            }
        }
    }
}

/// White breadboard with bevel, centre channel, striped rails and the batched hole grid.
pub fn breadboard() -> Geo {
    let k = Kit::new();
    let mut g = Geo::default();
    let (x0, x1) = BOARD_X;
    let cx = (x0 + x1) / 2.0;
    let w = x1 - x0;
    g.append(&gl::bevel_box(w, TOP, 2.0 * BOARD_Z, 0.12), &math::translate([cx, 0.0, 0.0]), [0.93, 0.93, 0.90]);
    let y = TOP + 0.004;
    k.flat(&mut g, [cx, y, 0.0], w - 0.7, 0.3, [0.72, 0.72, 0.70]);
    for (z, col) in [(-3.02, C_VCC), (-2.18, [0.15, 0.30, 0.85]), (2.18, C_SDA), (3.02, C_SCL)] {
        k.flat(&mut g, [cx, y, z], w - 0.9, 0.05, col);
    }
    let cols = ((w - 1.0) / PITCH) as usize;
    for c in 0..cols {
        let x = x0 + 0.5 + c as f32 * PITCH;
        for r in 0..5 {
            let z = ROW0 + r as f32 * PITCH;
            k.flat(&mut g, [x, y + 0.001, z], 0.1, 0.1, C_HOLE);
            k.flat(&mut g, [x, y + 0.001, -z], 0.1, 0.1, C_HOLE);
        }
        if c % 6 != 5 && c > 1 && c + 2 < cols {
            for z in [Z_VCC, Z_GND, Z_SDA, Z_SCL] {
                k.flat(&mut g, [x, y + 0.001, z], 0.1, 0.1, C_HOLE);
            }
        }
    }
    g
}

/// One pickable model: lit triangles, unlit emissive triangles (screens, LEDs), its bounding box
/// and a label anchor.
pub struct PartModel {
    pub hit: Hit,
    pub lit: Geo,
    pub glow: Geo,
    pub lo: V,
    pub hi: V,
    pub anchor: V,
    /// Sits on the breadboard (shadow on the board top) rather than on the desk.
    pub on_board: bool,
}

impl PartModel {
    fn new(hit: Hit, lit: Geo, glow: Geo, on_board: bool) -> PartModel {
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in lit.pos.chunks(3).chain(glow.pos.chunks(3)) {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        if lit.is_empty() && glow.is_empty() {
            (lo, hi) = ([0.0; 3], [0.0; 3]);
        }
        let anchor = [(lo[0] + hi[0]) / 2.0, hi[1] + 0.3, (lo[2] + hi[2]) / 2.0];
        PartModel { hit, lit, glow, lo, hi, anchor, on_board }
    }
}

/// Everything that depends on the board: part models and the SDA / SCL pulse paths.
pub struct Scene {
    pub parts: Vec<PartModel>,
    pub sda: Vec<V>,
    pub scl: Vec<V>,
    /// Pulses die at the end of the path (the bus fails there).
    pub stop: bool,
}

/// 5×7 bitmaps for the few glyphs the OLED shows.
fn glyph(c: char) -> [u8; 7] {
    match c {
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        '-' => [0, 0, 0, 0x1f, 0, 0, 0],
        _ => [0; 7],
    }
}

/// OLED text as lit pixels on a screen centred at `c` (width `w`).
fn screen_text(k: &Kit, g: &mut Geo, c: V, w: f32, s: &str, col: V) {
    let n = s.chars().count().max(1);
    let px = (w / (n as f32 * 6.0 + 2.0)).min(0.06);
    let x0 = c[0] - (n as f32 * 6.0 - 1.0) * px / 2.0;
    let z0 = c[2] - 3.0 * px;
    for (i, ch) in s.chars().enumerate() {
        for (r, bits) in glyph(ch).iter().enumerate() {
            for b in 0..5 {
                if bits >> (4 - b) & 1 == 1 {
                    let x = x0 + (i as f32 * 6.0 + b as f32 + 0.5) * px;
                    k.flat(g, [x, c[1], z0 + (r as f32 + 0.5) * px], px * 0.86, px * 0.86, col);
                }
            }
        }
    }
}

fn pcb_color(k: Kind) -> V {
    match k {
        Kind::Bme280 => [0.42, 0.20, 0.62],
        Kind::Mpu6050 => [0.10, 0.30, 0.72],
        Kind::Ssd1306 => [0.06, 0.06, 0.07],
        Kind::Vl53l0x => [0.62, 0.16, 0.45],
        Kind::Generic10k => [0.10, 0.45, 0.25],
        Kind::LevelShifter => [0.78, 0.14, 0.14],
        _ => [0.2, 0.2, 0.2],
    }
}

/// The four bus jumpers of a chip in slot centre `xc`: VCC / GND over to the back rails, SCL / SDA to
/// the front rails. `xs` are the pin columns in that order; `zb` the back-side start row.
fn bus_jumpers(k: &Kit, g: &mut Geo, xs: [f32; 4], zb: f32) {
    let zf = ROW0 + 4.0 * PITCH;
    let r = 0.055;
    k.wire(g, &arc([xs[0], TOP, zb], [xs[0], TOP, Z_VCC], if zb > 0.0 { 1.75 } else { 0.35 }), r, C_VCC);
    k.wire(g, &arc([xs[1], TOP, zb], [xs[1], TOP, Z_GND], if zb > 0.0 { 1.6 } else { 0.3 }), r, C_GND);
    k.wire(g, &arc([xs[2], TOP, zf], [xs[2], TOP, Z_SCL], 0.4), r, C_SCL);
    k.wire(g, &arc([xs[3], TOP, zf], [xs[3], TOP, Z_SDA], 0.3), r, C_SDA);
}

/// Sensor module / level shifter / bare chip in slot centre `xc`.
fn module(k: &Kit, p: &Part, xc: f32, status: Status, hot: bool) -> (Geo, Geo) {
    let (mut g, mut e) = (Geo::default(), Geo::default());
    let xs = [-1.5, -0.5, 0.5, 1.5].map(|i| xc + i * PITCH);
    if p.kind == Kind::BareChip {
        // DIP-8 straddling the centre channel.
        k.on(&mut g, [xc, TOP + 0.08, 0.0], [1.0, 0.3, 0.62], [0.09, 0.09, 0.10]);
        k.flat(&mut g, [xc - 0.38, TOP + 0.381, -0.18], 0.08, 0.08, [0.3, 0.3, 0.32]);
        for (i, x) in xs.iter().enumerate() {
            for s in [-1.0, 1.0] {
                k.on(&mut g, [*x, TOP - 0.2, s * ROW0], [0.07, 0.42, 0.05], C_SILVER);
                k.bx(&mut g, [*x, TOP + 0.2, s * 0.34], [0.08, 0.04, 0.1], C_SILVER);
            }
            let _ = i;
        }
        let zb = -(ROW0 + 4.0 * PITCH);
        bus_jumpers(k, &mut g, xs, zb);
        return (g, e);
    }
    // Header pins in the first row in front of the channel, PCB lying over the channel.
    k.header(&mut g, [xs[0], TOP, ROW0], 4, true);
    let pcb_y = TOP + 0.25;
    let (z0, z1) = (ROW0 + 0.22, ROW0 + 0.22 - 2.0);
    let zc = (z0 + z1) / 2.0;
    let col = pcb_color(p.kind);
    if p.kind == Kind::LevelShifter {
        k.on(&mut g, [xc, pcb_y, (z0 + zc) / 2.0], [1.7, 0.16, 1.0], col);
        k.on(&mut g, [xc, pcb_y, (zc + z1) / 2.0], [1.7, 0.16, 1.0], [0.12, 0.28, 0.78]);
        for i in 0..2 {
            k.on(&mut g, [xc - 0.35 + 0.7 * i as f32, pcb_y + 0.16, zc], [0.3, 0.1, 0.25], C_BLACK);
        }
    } else {
        k.on(&mut g, [xc, pcb_y, zc], [1.7, 0.16, 2.0], col);
    }
    let top = pcb_y + 0.16;
    // Gold pads of the header holes.
    for x in xs {
        k.flat(&mut g, [x, top + 0.002, ROW0], 0.14, 0.14, C_GOLD);
    }
    match p.kind {
        Kind::Bme280 => k.on(&mut g, [xc, top, zc - 0.2], [0.28, 0.1, 0.28], C_SILVER),
        Kind::Mpu6050 => {
            k.on(&mut g, [xc, top, zc - 0.2], [0.42, 0.09, 0.42], C_BLACK);
            k.on(&mut g, [xc + 0.5, top, zc - 0.6], [0.25, 0.1, 0.18], C_BLACK);
        }
        Kind::Ssd1306 => {
            k.on(&mut g, [xc, top, zc - 0.15], [1.62, 0.12, 1.25], [0.02, 0.03, 0.04]);
            let c = [xc, top + 0.125, zc - 0.15];
            k.flat(&mut e, c, 1.5, 1.05, [0.02, 0.05, 0.09]);
            let (txt, tc) = match status {
                Status::Works => ("I2C OK", [0.55, 0.95, 1.0]),
                Status::Unstable => ("I2C OK", [1.0, 0.72, 0.25]),
                Status::Fails => ("--", [0.45, 0.55, 0.6]),
            };
            screen_text(k, &mut e, [c[0], c[1] + 0.002, c[2]], 1.4, txt, tc);
        }
        Kind::Vl53l0x => {
            k.on(&mut g, [xc, top, zc - 0.2], [0.5, 0.1, 0.28], C_BLACK);
            k.flat(&mut e, [xc - 0.1, top + 0.102, zc - 0.2], 0.1, 0.1, [0.45, 0.06, 0.1]);
            k.flat(&mut e, [xc + 0.12, top + 0.102, zc - 0.2], 0.08, 0.08, [0.2, 0.05, 0.08]);
        }
        Kind::Generic10k => k.on(&mut g, [xc, top, zc - 0.2], [0.5, 0.1, 0.5], C_BLACK),
        _ => {}
    }
    if p.pullup_ohm().is_some() {
        // On-board SMD pull-ups (two: SDA and SCL), red when they cause the overlap problem.
        let body = if hot { C_RED } else { [0.05, 0.05, 0.05] };
        for i in 0..2 {
            let c = [xc - 0.55 + 0.25 * i as f32, top, zc + 0.6];
            k.on(&mut g, c, [0.1, 0.06, 0.2], body);
            for s in [-1.0, 1.0] {
                k.on(&mut g, [c[0], top, c[2] + s * 0.1], [0.11, 0.065, 0.04], C_SILVER);
            }
            if hot {
                k.flat(&mut e, [c[0], top + 0.07, c[2]], 0.12, 0.22, [1.0, 0.25, 0.2]);
            }
        }
    }
    bus_jumpers(k, &mut g, xs, ROW0 + 4.0 * PITCH);
    (g, e)
}

/// Through-hole pull-ups: one from VCC to SDA and one to SCL, bands matching the value.
fn resistors(k: &Kit, p: &Part, xc: f32) -> Geo {
    let mut g = Geo::default();
    let y = TOP + 0.55;
    for (dx, zr) in [(-0.4, Z_SDA), (0.4, Z_SCL)] {
        let x = xc + dx;
        let legs = [
            [x, TOP - 0.1, Z_VCC],
            [x, TOP + 0.3, Z_VCC],
            [x, y, Z_VCC + 0.45],
            [x, y, -0.4],
            [x, y, 1.6],
            [x, y, zr - 0.45],
            [x, TOP + 0.3, zr],
            [x, TOP - 0.1, zr],
        ];
        g.append(&gl::tube(&gl::catmull_rom(&legs, 4), 0.022, 5), &math::IDENTITY, C_SILVER);
        let c = [x, y, -0.15];
        g.append(&k.cap, &math::basis(X, Z, [0.0, -1.0, 0.0], [0.13, 0.13, 0.13], c), [0.86, 0.74, 0.55]);
        if let Some(d) = p.pullup_ohm().and_then(bands) {
            for (j, dg) in d.iter().enumerate() {
                k.cyl_z(&mut g, [x, y, c[2] - 0.2 + j as f32 * 0.11], 0.136, 0.055, BAND_RGB[*dg]);
            }
            k.cyl_z(&mut g, [x, y, c[2] + 0.22], 0.136, 0.05, C_GOLD);
        }
    }
    g
}

/// Jumper wire part: SDA and SCL loops whose size follows the chosen length.
fn wire_part(k: &Kit, p: &Part, xc: f32) -> Geo {
    let mut g = Geo::default();
    let s = loop_size(p.len_cm);
    k.wire(&mut g, &sda_loop(xc, p.len_cm), 0.06, C_SDA);
    let (a, b) = wire_ends(xc, Z_SCL, 0.3);
    k.wire(&mut g, &wire_loop(a, b, s * 0.92), 0.06, C_SCL);
    g
}

/// MCU board on the desk left of the breadboard; returns the model and its four header pin tops
/// (VCC, GND, SDA, SCL).
fn mcu(k: &Kit, kind: Kind) -> (Geo, Geo, [V; 4]) {
    let (mut g, mut e) = (Geo::default(), Geo::default());
    match kind {
        Kind::Esp32 => {
            let c = [-5.6, 0.0, 0.0];
            for s in [-1.0, 1.0] {
                k.on(&mut g, [c[0] + s * 1.15, 0.0, 0.0], [0.25, 0.85, 4.8], C_BLACK);
            }
            let y = 0.85;
            k.on(&mut g, [c[0], y, 0.0], [2.8, 0.16, 5.2], [0.07, 0.07, 0.08]);
            let t = y + 0.16;
            k.on(&mut g, [c[0], t, -1.2], [1.8, 0.32, 1.7], C_SILVER);
            k.on(&mut g, [c[0], t, -2.25], [1.8, 0.04, 0.6], [0.1, 0.1, 0.12]);
            for i in 0..4 {
                k.on(&mut g, [c[0] - 0.6 + 0.4 * i as f32, t + 0.04, -2.25], [0.08, 0.02, 0.45], C_GOLD);
            }
            k.on(&mut g, [c[0], t - 0.1, 2.45], [0.8, 0.3, 0.6], C_SILVER);
            for s in [-1.0, 1.0] {
                k.on(&mut g, [c[0] + s * 0.8, t, 1.9], [0.4, 0.15, 0.3], C_SILVER);
                k.cyl_y(&mut g, [c[0] + s * 0.8, t + 0.15, 1.9], 0.1, 0.06, C_BLACK);
            }
            k.on(&mut g, [c[0] + 0.3, t, 0.6], [0.5, 0.15, 0.5], C_BLACK);
            k.flat(&mut e, [c[0] - 0.5, t + 0.01, 1.3], 0.12, 0.08, [1.0, 0.15, 0.1]);
            for i in 0..19 {
                let z = -2.3 + i as f32 * PITCH;
                for s in [-1.0, 1.0] {
                    k.flat(&mut g, [c[0] + s * 1.15, t + 0.002, z], 0.12, 0.12, C_GOLD);
                }
            }
            let px = c[0] + 1.15;
            let pins = [[px, t, 1.8], [px, t, 1.8 - PITCH], [px, t, -0.5], [px, t, -0.5 + PITCH]];
            for p in pins {
                k.on(&mut g, p, [0.25, 0.9, 0.25], C_BLACK);
            }
            (g, e, pins.map(|p| [p[0], p[1] + 0.9, p[2]]))
        }
        Kind::RasPi => {
            let (cx, w, d) = (-6.3, 8.5, 5.6);
            for (sx, sz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                k.cyl_y(&mut g, [cx + sx * (w / 2.0 - 0.35), 0.0, sz * (d / 2.0 - 0.35)], 0.15, 0.25, [0.7, 0.7, 0.6]);
            }
            let y = 0.25;
            g.append(&gl::bevel_box(w, 0.16, d, 0.03), &math::translate([cx, y, 0.0]), [0.05, 0.45, 0.18]);
            let t = y + 0.16;
            let left = cx - w / 2.0;
            k.on(&mut g, [left + 1.0, t, -1.75], [2.1, 1.35, 1.6], C_SILVER);
            for z in [0.15, 1.95] {
                k.on(&mut g, [left + 0.85, t, z], [1.75, 1.55, 1.35], C_SILVER);
                for dy in [0.35, 0.95] {
                    k.bx(&mut g, [left - 0.02, t + dy, z], [0.04, 0.32, 1.0], [0.12, 0.12, 0.14]);
                }
            }
            k.on(&mut g, [cx + 0.3, t, -0.2], [1.4, 0.12, 1.4], C_SILVER);
            k.on(&mut g, [cx + 0.3, t + 0.12, -0.2], [1.1, 0.02, 1.1], [0.55, 0.57, 0.6]);
            k.on(&mut g, [cx + 2.2, t, -0.6], [1.0, 0.12, 0.8], C_BLACK);
            // 40-pin header along the front edge, pin 1 at the right end.
            let hz = d / 2.0 - 0.45;
            let hx0 = cx + 3.6 - 19.0 * PITCH;
            k.on(&mut g, [hx0 + 9.5 * PITCH - PITCH / 2.0, t, hz], [20.0 * PITCH, 0.25, 2.0 * PITCH], C_BLACK);
            for i in 0..20 {
                for r in [-0.5, 0.5] {
                    k.on(&mut g, [hx0 + i as f32 * PITCH, t, hz + r * PITCH], [0.064, 0.85, 0.064], C_GOLD);
                }
            }
            k.flat(&mut e, [left + 0.6, t + 0.01, -2.4], 0.14, 0.08, [1.0, 0.15, 0.1]);
            k.flat(&mut e, [left + 0.9, t + 0.01, -2.4], 0.14, 0.08, [0.2, 1.0, 0.3]);
            let x1 = cx + 3.6;
            let (zi, zo) = (hz - 0.5 * PITCH, hz + 0.5 * PITCH);
            let pins = [[x1, t, zi], [x1 - 2.0 * PITCH, t, zo], [x1 - PITCH, t, zi], [x1 - 2.0 * PITCH, t, zi]];
            for p in pins {
                k.on(&mut g, [p[0], t + 0.85, p[2]], [0.24, 1.0, 0.24], C_BLACK);
            }
            (g, e, pins.map(|p| [p[0], t + 1.85, p[2]]))
        }
        _ => {
            // Arduino Uno: blue PCB on standoffs, USB-B, DC jack, female headers, DIP-28.
            let (cx, w, d) = (-5.2, 6.86, 5.34);
            for (sx, sz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                k.cyl_y(&mut g, [cx + sx * (w / 2.0 - 0.4), 0.0, sz * (d / 2.0 - 0.4)], 0.15, 0.25, C_BLACK);
            }
            let y = 0.25;
            g.append(&gl::bevel_box(w, 0.16, d, 0.03), &math::translate([cx, y, 0.0]), [0.0, 0.42, 0.60]);
            let t = y + 0.16;
            let left = cx - w / 2.0;
            k.on(&mut g, [left + 0.55, t, -1.35], [1.6, 1.1, 1.2], C_SILVER);
            k.on(&mut g, [left + 0.5, t, 1.75], [1.4, 1.1, 0.9], C_BLACK);
            k.socket(&mut g, [cx + 0.2, t, -d / 2.0 + 0.2], 10, 1, C_BLACK);
            k.socket(&mut g, [cx + 2.3, t, -d / 2.0 + 0.2], 8, 1, C_BLACK);
            k.socket(&mut g, [cx - 0.3, t, d / 2.0 - 0.2], 8, 1, C_BLACK);
            k.socket(&mut g, [cx + 2.0, t, d / 2.0 - 0.2], 6, 1, C_BLACK);
            let (chx, chz) = (cx + 1.2, 1.1);
            k.on(&mut g, [chx, t + 0.12, chz], [3.5, 0.35, 0.8], [0.09, 0.09, 0.1]);
            for i in 0..14 {
                for s in [-1.0, 1.0] {
                    k.bx(&mut g, [chx - 1.65 + i as f32 * PITCH, t + 0.12, chz + s * 0.43], [0.06, 0.24, 0.08], C_SILVER);
                }
            }
            k.on(&mut g, [cx - 0.8, t, -0.4], [0.35, 0.2, 0.9], C_SILVER);
            k.on(&mut g, [cx - 1.6, t, -2.0], [0.5, 0.25, 0.5], [0.85, 0.2, 0.2]);
            k.flat(&mut e, [cx + 0.9, t + 0.01, -0.9], 0.12, 0.08, [0.3, 1.0, 0.35]);
            let pz = d / 2.0 - 0.2;
            let px = |i: usize, c0: f32, n: usize| c0 - n as f32 * PITCH / 2.0 + (i as f32 + 0.5) * PITCH;
            let pins = [[px(4, cx - 0.3, 8), t, pz], [px(5, cx - 0.3, 8), t, pz], [px(4, cx + 2.0, 6), t, pz], [px(5, cx + 2.0, 6), t, pz]];
            for p in pins {
                k.on(&mut g, [p[0], t + 0.85, p[2]], [0.24, 1.0, 0.24], C_BLACK);
            }
            (g, e, pins.map(|p| [p[0], t + 1.85, p[2]]))
        }
    }
}

/// Rail entry hole for each MCU jumper (VCC, GND, SDA, SCL).
fn rail_entry(i: usize) -> V {
    let z = [Z_VCC, Z_GND, Z_SDA, Z_SCL][i];
    [RAIL_IN + if i % 2 == 1 { PITCH } else { 0.0 }, TOP, z]
}

fn mcu_jumper(start: V, i: usize) -> Vec<V> {
    arc(start, rail_entry(i), 1.4)
}

/// x where the SDA / SCL pulses stop: the culprit part, or the rail entry when the bus fails with no
/// part to blame. `None` when the bus works (pulses run to the last part).
fn stop_x(b: &Board, v: &Verdict, hot: &[Hit]) -> Option<f32> {
    if v.status != Status::Fails {
        return None;
    }
    Some(match hot.iter().find_map(|h| if let Hit::Part(i) = h { Some(*i) } else { None }) {
        Some(i) => slot_center(i),
        None if b.parts.is_empty() => RAIL_IN,
        None => RAIL_IN + 0.6,
    })
}

/// Build the board-dependent scene.
pub fn build(b: &Board, v: &Verdict) -> Scene {
    let k = Kit::new();
    let hot = culprits(b, v);
    let overlap = matches!(v.primary(), Some(Issue::TooStrong { .. }));
    let mut parts = vec![];
    let mut starts = None;
    if let Some(m) = b.mcu {
        let (mut g, e, pins) = mcu(&k, m.kind);
        for (i, p) in pins.iter().enumerate() {
            k.wire(&mut g, &mcu_jumper(*p, i), 0.06, [C_VCC, C_GND, C_SDA, C_SCL][i]);
        }
        starts = Some(pins);
        parts.push(PartModel::new(Hit::Mcu, g, e, false));
    }
    for (i, p) in b.parts.iter().enumerate() {
        let xc = slot_center(i);
        let hit = Hit::Part(i);
        let (g, e) = match p.kind {
            Kind::PullUp => (resistors(&k, p, xc), Geo::default()),
            Kind::Wire => (wire_part(&k, p, xc), Geo::default()),
            _ => module(&k, p, xc, v.status, overlap && p.pullup_ohm().is_some() && hot.contains(&hit)),
        };
        parts.push(PartModel::new(hit, g, e, p.kind != Kind::Wire));
    }
    let (mut sda, mut scl) = (vec![], vec![]);
    if let Some(pins) = starts {
        let end = stop_x(b, v, &hot).unwrap_or(match b.parts.len() {
            0 => RAIL_IN + 0.6,
            n => slot_center(n - 1) + 0.6,
        });
        for (line, i, z) in [(&mut sda, 2, Z_SDA), (&mut scl, 3, Z_SCL)] {
            line.extend(mcu_jumper(pins[i], i).into_iter().skip(1));
            let x0 = rail_entry(i)[0];
            let y = TOP + 0.09;
            line.push([x0, y, z]);
            if end > x0 {
                line.push([end, y, z]);
            }
        }
    }
    Scene { parts, sda, scl, stop: v.status == Status::Fails }
}

/// Camera view presets.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ViewPreset {
    Persp,
    Top,
    Side,
}

/// Orbit for a preset that frames the MCU and the used part of the board (`n_parts` placed) in a
/// viewport of the given aspect ratio.
pub fn preset(p: ViewPreset, aspect: f32, n_parts: usize) -> Orbit {
    let x0 = -10.8;
    let x1 = (SLOT_X0 + n_parts.clamp(3, MAX_PARTS) as f32 * SLOT_W + 1.0).min(BOARD_X.1 + 0.4);
    let half_w = (x1 - x0) / 2.0 + 0.6;
    let cx = (x0 + x1) / 2.0;
    let tan_h = (FOVY / 2.0).tan() * aspect.max(0.3);
    // Wide screens are limited by the board depth instead of its width.
    // Narrow (phone) screens get extra room: the perspective yaw pushes the far end outward.
    let narrow = 1.0 + 0.3 * (1.5 - aspect).clamp(0.0, 1.0);
    let fit = |k: f32| (half_w / tan_h).max(8.5 / (FOVY / 2.0).tan()) * k * narrow;
    match p {
        ViewPreset::Persp => Orbit { target: [cx + 1.0, 0.8, 0.8], yaw: 0.28, pitch: 0.70, dist: fit(0.98).clamp(18.0, 120.0) },
        ViewPreset::Top => Orbit { target: [cx, 0.0, 0.8], yaw: 0.0, pitch: math::PITCH_MAX, dist: fit(0.98).clamp(22.0, 120.0) },
        ViewPreset::Side => Orbit { target: [cx, 1.5, 0.0], yaw: 0.0, pitch: 0.12, dist: fit(0.95).clamp(18.0, 120.0) },
    }
}

/// View-projection matrix for an orbit pose.
pub fn view_proj(o: &Orbit, aspect: f32) -> (M4, M4) {
    let view = math::look_at(o.eye(), o.target, [0.0, 1.0, 0.0]);
    let proj = math::perspective(FOVY, aspect.max(0.1), 0.5, 600.0);
    (view, proj)
}

/// Where a screen ray (NDC) meets the breadboard top plane.
pub fn pick_board(inv_vp: &M4, x: f32, y: f32) -> Option<V> {
    let (o, d) = math::ndc_ray(inv_vp, x, y);
    math::ray_plane(o, d, [0.0, TOP, 0.0], [0.0, 1.0, 0.0])
}

/// Nearest part under a screen ray.
pub fn pick_part(parts: &[(Hit, V, V)], inv_vp: &M4, x: f32, y: f32) -> Option<Hit> {
    let (o, d) = math::ndc_ray(inv_vp, x, y);
    parts
        .iter()
        .filter_map(|(h, lo, hi)| math::ray_aabb(o, d, *lo, *hi).map(|t| (t, *h)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, h)| h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tube_has_expected_vertices_and_closed_ends() {
        let path = gl::catmull_rom(&[[0.0; 3], [1.0, 1.0, 0.0], [2.0, 0.0, 0.5]], 6);
        let sides = 6;
        let g = gl::tube(&path, 0.1, sides);
        assert_eq!(g.len(), path.len() * sides + 2 * (sides + 1));
        assert_eq!(g.idx.len(), (path.len() - 1) * sides * 6 + 2 * sides * 3);
        assert!(g.idx.iter().all(|i| (*i as usize) < g.len()));
        // Cap centres sit on the path ends; every ring vertex is `r` from its path point.
        let v = |i: usize| -> V { [g.pos[i * 3], g.pos[i * 3 + 1], g.pos[i * 3 + 2]] };
        let caps = path.len() * sides;
        assert_eq!(v(caps), path[0]);
        assert_eq!(v(caps + sides + 1), path[path.len() - 1]);
        for (i, p) in path.iter().enumerate() {
            for s in 0..sides {
                let d = math::sub(v(i * sides + s), *p);
                assert!((math::dot(d, d).sqrt() - 0.1).abs() < 1e-4);
            }
        }
        assert!(gl::tube(&path[..1], 0.1, 6).is_empty());
    }

    #[test]
    fn resistor_bands_match_e12_values() {
        assert_eq!(bands(4700.0), Some([4, 7, 2])); // yellow violet red
        assert_eq!(bands(10_000.0), Some([1, 0, 3])); // brown black orange
        assert_eq!(bands(2200.0), Some([2, 2, 2])); // red red red
        assert_eq!(bands(1800.0), Some([1, 8, 2])); // brown grey red
        assert_eq!(bands(999.6), Some([1, 0, 2]));
        assert_eq!(bands(f64::INFINITY), None);
        let p = Kind::PullUp.part().with_r(4.7);
        assert_eq!(p.pullup_ohm().and_then(bands), Some([4, 7, 2]));
    }

    #[test]
    fn slots_do_not_overlap_and_fit_the_board() {
        for n in 0..=MAX_PARTS + 2 {
            let s = slot_layout(n);
            assert_eq!(s.len(), n.min(MAX_PARTS));
            for w in s.windows(2) {
                assert!(w[0].1 < w[1].0, "{n}: {w:?}");
            }
            for (i, (a, b)) in s.iter().enumerate() {
                assert!(*a > BOARD_X.0 && *b < BOARD_X.1);
                assert!(slot_center(i) > *a && slot_center(i) < *b);
            }
        }
        // A full board of modules: part boxes stay apart.
        let mut b = Board { parts: vec![Kind::Mpu6050.part(); MAX_PARTS], ..Board::default() };
        b.parts[3] = Kind::PullUp.part();
        let sc = build(&b, &b.verdict());
        let boxes: Vec<_> = sc.parts.iter().filter(|p| p.hit != Hit::Mcu).map(|p| (p.lo[0], p.hi[0])).collect();
        for w in boxes.windows(2) {
            assert!(w[0].1 < w[1].0, "{w:?}");
        }
    }

    #[test]
    fn pick_ray_hits_board_where_expected() {
        for p in [ViewPreset::Persp, ViewPreset::Top, ViewPreset::Side] {
            let (v, pr) = view_proj(&preset(p, 1.6, 8), 1.6);
            let vp = math::mul(&pr, &v);
            let inv = math::invert(&vp).unwrap();
            let want = [SLOT_X0 + 3.0 * SLOT_W + 0.1, TOP, 1.0];
            let s = math::project(&vp, want).unwrap();
            let hit = pick_board(&inv, s[0], s[1]).unwrap();
            assert!(math::dot(math::sub(hit, want), math::sub(hit, want)).sqrt() < 0.02, "{p:?}: {hit:?}");
            let b = Board { parts: vec![Kind::Bme280.part(); 5], ..Board::default() };
            assert_eq!(drop_slot(&b, hit[0]), 3);
        }
        // Screen centre of the top view lands on the target; a part under the ray is picked.
        let o = preset(ViewPreset::Top, 1.0, 4);
        let (v, pr) = view_proj(&o, 1.0);
        let inv = math::invert(&math::mul(&pr, &v)).unwrap();
        let c = pick_board(&inv, 0.0, 0.0).unwrap();
        assert!((c[0] - o.target[0]).abs() < 0.05 && (c[2] - o.target[2]).abs() < 0.05);
        let boxes = [(Hit::Part(0), [c[0] - 0.5, 0.0, c[2] - 0.5], [c[0] + 0.5, 2.0, c[2] + 0.5])];
        assert_eq!(pick_part(&boxes, &inv, 0.0, 0.0), Some(Hit::Part(0)));
        assert_eq!(pick_part(&boxes, &inv, 0.9, 0.9), None);
    }

    #[test]
    fn wire_arc_grows_with_length() {
        let l: Vec<f32> = WIRE_LENGTHS_CM.iter().map(|c| gl::path_len(&sda_loop(slot_center(2), *c))).collect();
        for w in l.windows(2) {
            assert!(w[1] > w[0] * 1.05, "{l:?}");
        }
        assert!(l[4] > 3.0 * l[0]);
        // The built wire part grows too.
        let size = |cm| {
            let b = Board { parts: vec![Kind::Wire.part().with_len(cm)], ..Board::default() };
            let p = &build(&b, &b.verdict()).parts[1];
            p.hi[2] - p.lo[2]
        };
        assert!(size(50.0) > size(10.0) && size(10.0) > size(5.0));
    }

    #[test]
    fn culprit_matches_verdict_part() {
        let p = presets();
        // ESP32 stacked pull-ups: the MPU6050 (slot 1) is to blame, its SMD pull-ups turn red.
        let (b, v) = (&p[1].2, p[1].2.verdict());
        assert_eq!(culprits(b, &v), vec![Hit::Part(1)]);
        let sc = build(b, &v);
        assert!(sc.stop);
        assert!(sc.parts[2].lit.col.chunks(3).any(|c| c == C_RED), "red SMD pull-ups");
        assert!(!sc.parts[3].lit.col.chunks(3).any(|c| c == C_RED));
        // Pulses stop at the culprit slot.
        assert!((sc.sda.last().unwrap()[0] - slot_center(1)).abs() < 1e-5);
        // 5 V hazard: the bare chip (slot 2).
        let (b, v) = (&p[3].2, p[3].2.verdict());
        assert_eq!(culprits(b, &v), vec![Hit::Part(2)]);
        assert_eq!(issue_kind(v.primary()), Some(Kind::BareChip));
        // Working and unstable boards blame nothing and pulses run to the last part.
        for i in [0, 2] {
            let (b, v) = (&p[i].2, p[i].2.verdict());
            assert!(culprits(b, &v).is_empty());
            let sc = build(b, &v);
            assert!(!sc.stop);
            assert!((sc.scl.last().unwrap()[0] - (slot_center(b.parts.len() - 1) + 0.6)).abs() < 1e-5);
        }
        // Raspberry Pi with too-strong pull-ups only from itself + a module: the MCU can be the culprit.
        let b = Board { mcu: Some(Kind::RasPi.part()), parts: vec![], ..Board::default() };
        assert!(culprits(&b, &b.verdict()).is_empty());
    }

    #[test]
    fn scene_builds_for_every_kind_and_empty_boards() {
        for k in Kind::PALETTE {
            let mut b = Board::default();
            b.add(k.part(), None);
            let sc = build(&b, &b.verdict());
            for p in &sc.parts {
                assert!(p.lit.len() < u16::MAX as usize && p.glow.len() < u16::MAX as usize, "{k:?}");
                assert!(p.lit.pos.iter().all(|x| x.is_finite()));
                assert!(p.lo.iter().zip(p.hi).all(|(a, b)| *a <= b));
            }
        }
        let empty = Board { mcu: None, parts: vec![], ..Board::default() };
        let sc = build(&empty, &empty.verdict());
        assert!(sc.parts.is_empty() && sc.sda.is_empty());
        assert!(breadboard().len() < u16::MAX as usize);
    }
}
