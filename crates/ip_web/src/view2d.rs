//! Canvas-2D breadboard: the fallback board view when WebGL2 is unavailable.
use super::*;

// Breadboard drawing, in logical units (canvas scales to its CSS width).
const BW: f64 = 1000.0;
const SLOT_X0: f64 = 200.0;
const SLOT_W: f64 = 98.0;
const Y_VCC: f64 = 34.0;
const Y_GND: f64 = 52.0;
const Y_SDA: f64 = 262.0;
const Y_SCL: f64 = 280.0;
const C_VCC: &str = "#e8433f";
const C_GND: &str = "#2f5fd0";
const C_SDA: &str = "#e0a91c";
const C_SCL: &str = "#1aa6b8";
const C_INK: &str = "#3a3326";
/// Wire loop length per cm, so a 50 cm jumper visibly hangs longer than a 5 cm one.
const LOOP_PX_PER_CM: f64 = 5.0;

// ---------- breadboard ----------

pub(crate) fn board_scale() -> f64 {
    el("board").map(|e| e.client_width() as f64 / BW).filter(|s| *s > 0.0).unwrap_or(1.0)
}

fn slot_x(i: usize) -> f64 {
    SLOT_X0 + i as f64 * SLOT_W
}

pub(crate) fn hit_test(b: &Board, lx: f64, ly: f64) -> Option<Hit> {
    if (12.0..=176.0).contains(&lx) && (66.0..=254.0).contains(&ly) && b.mcu.is_some() {
        return Some(Hit::Mcu);
    }
    if lx >= SLOT_X0 && ly >= 14.0 {
        let i = ((lx - SLOT_X0) / SLOT_W).floor() as usize;
        if i < b.parts.len() {
            return Some(Hit::Part(i));
        }
    }
    None
}

pub(crate) fn drop_slot(b: &Board, lx: f64) -> usize {
    (((lx - SLOT_X0) / SLOT_W + 0.5).floor().max(0.0) as usize).min(b.parts.len())
}

pub(crate) fn short_name(k: Kind, en: bool) -> &'static str {
    match k {
        Kind::Uno => "UNO",
        Kind::Esp32 => "ESP32",
        Kind::RasPi => "Pi",
        Kind::Bme280 => "BME280",
        Kind::Mpu6050 => "MPU6050",
        Kind::Ssd1306 => "OLED",
        Kind::Vl53l0x => "VL53L0X",
        Kind::Generic10k => tr(en, "10k 모듈", "10k mod."),
        Kind::BareChip => "3.3V IC",
        Kind::LevelShifter => "LV↔HV",
        Kind::PullUp => tr(en, "풀업", "pull-up"),
        Kind::Wire => tr(en, "점퍼선", "jumper"),
    }
}

fn body_color(k: Kind) -> &'static str {
    match k {
        Kind::Uno => "#0f7c8a",
        Kind::Esp32 => "#23262e",
        Kind::RasPi => "#2b8a3e",
        Kind::Bme280 => "#6b3fa0",
        Kind::Mpu6050 => "#1f5fa8",
        Kind::Ssd1306 => "#1b1f2a",
        Kind::Vl53l0x => "#7a2d5a",
        Kind::Generic10k => "#2f7d4a",
        Kind::LevelShifter => "#b33a3a",
        _ => "#222",
    }
}

const BAND: [&str; 10] = ["#111", "#7a4a1c", "#d22", "#f07a10", "#f2d21b", "#2a9a3a", "#2255cc", "#7a3fb0", "#888", "#f4f4f4"];

fn resistor(ctx: &Ctx, x: f64, y0: f64, y1: f64, ohm: f64) {
    seg(ctx, (x, y0), (x, y1), "#9a9a9a", 2.0);
    dot(ctx, x, y0, 3.0, "#666");
    dot(ctx, x, y1, 3.0, "#666");
    let (top, h) = (118.0, 52.0);
    rect(ctx, x - 8.0, top, 16.0, h, "#d9b98a");
    if let Some(digits) = crate::scene::bands(ohm) {
        for (j, dg) in digits.iter().enumerate() {
            rect(ctx, x - 8.0, top + 7.0 + j as f64 * 10.0, 16.0, 5.0, BAND[*dg]);
        }
        rect(ctx, x - 8.0, top + h - 11.0, 16.0, 4.0, "#c9a227");
    }
}

/// U-shaped jumper hanging from `y` between `xa` and `xb`, with a path length of `len_px`.
fn wire_loop(ctx: &Ctx, xa: f64, xb: f64, y: f64, len_px: f64, color: &str) -> f64 {
    let r = (xb - xa) / 2.0;
    let drop = ((len_px - PI * r) / 2.0).max(2.0);
    ctx.begin_path();
    ctx.move_to(xa, y);
    ctx.line_to(xa, y + drop);
    let _ = ctx.arc_with_anticlockwise(xa + r, y + drop, r, PI, 0.0, true);
    ctx.line_to(xb, y);
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(4.0);
    ctx.stroke();
    dot(ctx, xa, y, 3.0, color);
    dot(ctx, xb, y, 3.0, color);
    y + drop + r
}

pub(crate) fn draw_board(app: &App) {
    let Some((ctx, w, _)) = ctx_for("board") else { return };
    let s = w / BW;
    let _ = ctx.scale(s, s);
    let en = app.en;
    let b = &app.board;
    // Board body, terminal holes, centre channel.
    rect(&ctx, 190.0, 14.0, 800.0, 286.0, "#ece6d6");
    rect(&ctx, 190.0, 152.0, 800.0, 8.0, "#d6cdb6");
    ctx.set_fill_style_str("rgba(60,50,30,0.32)");
    for row in 0..10 {
        let y = 80.0 + row as f64 * 15.0 + if row >= 5 { 12.0 } else { 0.0 };
        for col in 0..30 {
            ctx.fill_rect(206.0 + col as f64 * 26.5, y, 4.0, 4.0);
        }
    }
    for (y, c, lbl) in [(Y_VCC, C_VCC, "VCC +"), (Y_GND, C_GND, "GND −"), (Y_SDA, C_SDA, "SDA"), (Y_SCL, C_SCL, "SCL")] {
        seg(&ctx, (196.0, y), (986.0, y), c, 3.0);
        text(&ctx, lbl, 984.0, y - 5.0, "bold 12px system-ui, sans-serif", c, "right");
    }
    let hot = issue_kind(app.v.primary());
    draw_mcu(&ctx, app, en);
    // Slots: parts, then empty spots for dropping.
    for (i, p) in b.parts.iter().enumerate() {
        let x0 = slot_x(i);
        draw_part(&ctx, p, x0, en);
        let hit = Some(Hit::Part(i));
        if hot == Some(p.kind) {
            outline(&ctx, (x0 + 4.0, 88.0, 90.0, 116.0), C_BAD, 3.0, true);
        }
        if app.sel == hit {
            outline(&ctx, (x0 + 2.0, 84.0, 94.0, 124.0), C_ACC, 3.0, false);
        }
        if app.hover == hit && app.drag.is_none() {
            outline(&ctx, (x0 + 4.0, 88.0, 90.0, 116.0), C_BAD, 2.0, false);
            dot(&ctx, x0 + 90.0, 90.0, 10.0, C_BAD);
            text(&ctx, "×", x0 + 90.0, 95.0, "bold 15px system-ui, sans-serif", "#fff", "center");
        }
    }
    for i in b.parts.len()..MAX_PARTS {
        let x0 = slot_x(i);
        outline(&ctx, (x0 + 10.0, 96.0, 78.0, 100.0), "rgba(58,51,38,0.28)", 1.5, true);
        if i == b.parts.len() {
            text(&ctx, "+", x0 + 49.0, 154.0, "28px system-ui, sans-serif", "rgba(58,51,38,0.35)", "center");
        }
    }
    if let Some(Drag { active: true, slot: Some(k), kind, .. }) = app.drag {
        if !kind.is_mcu() && b.parts.len() < MAX_PARTS {
            if k < b.parts.len() {
                rect(&ctx, slot_x(k) - 3.0, 86.0, 6.0, 120.0, C_ACC);
            } else {
                outline(&ctx, (slot_x(k) + 8.0, 94.0, 82.0, 104.0), C_ACC, 3.0, false);
            }
        }
    }
    if app.hover.is_none() && app.drag.is_none() && b.parts.is_empty() {
        text(&ctx, tr(en, "부품을 이쪽으로 끌어오거나 팔레트에서 누르세요", "Drag parts here or tap them in the palette"),
             590.0, 330.0, "16px system-ui, sans-serif", C_TEXT, "center");
    }
}

fn draw_mcu(ctx: &Ctx, app: &App, en: bool) {
    let (x, y, w, h) = (14.0, 70.0, 158.0, 180.0);
    let Some(m) = app.board.mcu else {
        outline(ctx, (x, y, w, h), C_TEXT, 2.0, true);
        text(ctx, tr(en, "MCU 없음", "No MCU"), x + w / 2.0, y + h / 2.0, "bold 16px system-ui, sans-serif", C_TEXT, "center");
        return;
    };
    rect(ctx, x, y, w, h, body_color(m.kind));
    if m.kind == Kind::Esp32 {
        rect(ctx, x + 34.0, y + 14.0, 90.0, 54.0, "#9aa3ad");
    }
    if m.kind == Kind::Uno {
        rect(ctx, x + 6.0, y + 12.0, 34.0, 26.0, "#c9ccd1");
    }
    text(ctx, short_name(m.kind, en), x + w / 2.0, y + 102.0, "bold 26px system-ui, sans-serif", "#fff", "center");
    text(ctx, &format!("{}V {}", m.kind.logic_v().unwrap_or(3.3), tr(en, "로직", "logic")), x + w / 2.0, y + 126.0,
         "14px system-ui, sans-serif", "rgba(255,255,255,0.85)", "center");
    if let Some(o) = m.pullup_ohm() {
        text(ctx, &format!("{} {}", tr(en, "풀업 내장", "on-board pull-up"), fmt_ohm(o)), x + w / 2.0, y + 146.0,
             "12px system-ui, sans-serif", "rgba(255,255,255,0.8)", "center");
    }
    // Header pins and jumpers to the four lines.
    for (py, ry, c, l) in [(110.0, Y_VCC, C_VCC, "V"), (130.0, Y_GND, C_GND, "G"), (190.0, Y_SDA, C_SDA, "D"), (210.0, Y_SCL, C_SCL, "C")] {
        ctx.begin_path();
        ctx.move_to(x + w, py);
        ctx.bezier_curve_to(188.0, py, 186.0, ry, 202.0, ry);
        ctx.set_stroke_style_str(c);
        ctx.set_line_width(3.5);
        ctx.stroke();
        dot(ctx, x + w, py, 3.5, "#ddd");
        text(ctx, l, x + w - 8.0, py + 4.0, "bold 11px system-ui, sans-serif", "#fff", "right");
    }
    let hit = Some(Hit::Mcu);
    if app.sel == hit {
        outline(ctx, (x - 4.0, y - 4.0, w + 8.0, h + 8.0), C_ACC, 3.0, false);
    }
    if app.hover == hit && app.drag.is_none() {
        outline(ctx, (x - 2.0, y - 2.0, w + 4.0, h + 4.0), C_BAD, 2.0, false);
    }
    if let Some(Drag { active: true, kind, .. }) = app.drag {
        if kind.is_mcu() {
            outline(ctx, (x - 4.0, y - 4.0, w + 8.0, h + 8.0), C_ACC, 3.0, false);
        }
    }
}

fn draw_part(ctx: &Ctx, p: &Part, x0: f64, en: bool) {
    let cx = x0 + 49.0;
    let k = p.kind;
    match k {
        Kind::PullUp => {
            let o = p.pullup_ohm().unwrap_or(f64::INFINITY);
            resistor(ctx, x0 + 34.0, Y_VCC, Y_SDA, o);
            resistor(ctx, x0 + 64.0, Y_VCC, Y_SCL, o);
            text(ctx, &fmt_ohm(o), cx, 214.0, "bold 14px system-ui, sans-serif", C_INK, "center");
            text(ctx, tr(en, "풀업", "pull-up"), cx, 232.0, "12px system-ui, sans-serif", C_INK, "center");
        }
        Kind::Wire => {
            let len = p.len_cm * LOOP_PX_PER_CM;
            let b1 = wire_loop(ctx, x0 + 28.0, x0 + 70.0, Y_SDA, len, C_SDA);
            let b2 = wire_loop(ctx, x0 + 36.0, x0 + 62.0, Y_SCL, len, C_SCL);
            text(ctx, tr(en, "점퍼선", "jumper"), cx, 140.0, "13px system-ui, sans-serif", C_INK, "center");
            text(ctx, &format!("{} cm", fmt_in(p.len_cm)), cx, 162.0, "bold 17px system-ui, sans-serif", C_INK, "center");
            text(ctx, &format!("{} cm", fmt_in(p.len_cm)), cx, (b1.max(b2) + 16.0).min(414.0), "12px system-ui, sans-serif", C_TEXT, "center");
        }
        _ => {
            let pins = [(x0 + 30.0, Y_VCC, C_VCC, true), (x0 + 44.0, Y_GND, C_GND, true), (x0 + 54.0, Y_SDA, C_SDA, false), (x0 + 68.0, Y_SCL, C_SCL, false)];
            for (px, ry, c, top) in pins {
                let py = if top { 96.0 } else { 196.0 };
                seg(ctx, (px, py), (px, ry), c, 3.0);
                dot(ctx, px, ry, 3.0, c);
            }
            let light = k == Kind::BareChip;
            if light {
                rect(ctx, x0 + 24.0, 112.0, 50.0, 62.0, "#1a1a1a");
                for j in 0..4 {
                    rect(ctx, x0 + 18.0, 118.0 + j as f64 * 14.0, 6.0, 4.0, "#aaa");
                    rect(ctx, x0 + 74.0, 118.0 + j as f64 * 14.0, 6.0, 4.0, "#aaa");
                }
            } else {
                rect(ctx, x0 + 10.0, 96.0, 78.0, 100.0, body_color(k));
                if k == Kind::Ssd1306 {
                    rect(ctx, x0 + 16.0, 102.0, 66.0, 22.0, "#0b2a4a");
                }
            }
            let (fg, fg2) = ("#fff", "rgba(255,255,255,0.82)");
            text(ctx, short_name(k, en), cx, 140.0, "bold 14px system-ui, sans-serif", fg, "center");
            let pu = p.pullup_ohm().map(fmt_ohm).unwrap_or_else(|| tr(en, "풀업 없음", "no pull-up").into());
            let small = "11px system-ui, sans-serif";
            if light {
                text(ctx, "3.3V", cx, 158.0, small, fg2, "center");
                text(ctx, &pu, cx, 190.0, small, C_INK, "center");
            } else {
                text(ctx, &pu, cx, 158.0, small, fg2, "center");
                if let Some(a) = p.addr {
                    text(ctx, &format!("0x{a:02X}"), cx, 174.0, small, fg2, "center");
                }
                if !p.five_v_ok {
                    text(ctx, tr(en, "3.3V 전용", "3.3 V only"), cx, 189.0, small, C_WARN, "center");
                }
            }
        }
    }
}
