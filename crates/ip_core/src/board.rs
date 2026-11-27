//! Breadboard model: parts snap onto VCC/GND/SDA/SCL, and the board turns into a [`Design`]
//! plus a plain-language verdict (works / unstable / fails, one reason, one fix).
//!
//! Simplifications (documented in the README): every pull-up goes to the MCU logic rail, all
//! parts sit on one bus segment, a level shifter only clears the 5 V hazard and adds its own
//! pull-ups and pin capacitance. Part values are typical illustrative presets, editable.

use crate::{analyze, e12_pick, grade, parallel, Design, Level, NumErr, Report, Speed};

/// Most parts that fit next to the MCU on the half-size board.
pub const MAX_PARTS: usize = 8;
/// Total bus capacitance that always fails, as stated for hobby boards.
pub const CB_FAIL_PF: f64 = 400.0;
/// Total jumper length above which a 400 kHz (or faster) bus is flagged unstable.
pub const WIRE_WARN_CM: f64 = 50.0;
/// Jumper lengths offered in the palette.
pub const WIRE_LENGTHS_CM: [f64; 5] = [5.0, 10.0, 20.0, 30.0, 50.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Uno,
    Esp32,
    RasPi,
    Bme280,
    Mpu6050,
    Ssd1306,
    Vl53l0x,
    Generic10k,
    BareChip,
    PullUp,
    Wire,
    LevelShifter,
}

impl Kind {
    pub const PALETTE: [Kind; 12] = [
        Kind::Uno,
        Kind::Esp32,
        Kind::RasPi,
        Kind::Bme280,
        Kind::Mpu6050,
        Kind::Ssd1306,
        Kind::Vl53l0x,
        Kind::Generic10k,
        Kind::BareChip,
        Kind::PullUp,
        Kind::Wire,
        Kind::LevelShifter,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Kind::Uno => "uno",
            Kind::Esp32 => "esp32",
            Kind::RasPi => "raspi",
            Kind::Bme280 => "bme280",
            Kind::Mpu6050 => "mpu6050",
            Kind::Ssd1306 => "ssd1306",
            Kind::Vl53l0x => "vl53l0x",
            Kind::Generic10k => "generic10k",
            Kind::BareChip => "barechip",
            Kind::PullUp => "pullup",
            Kind::Wire => "wire",
            Kind::LevelShifter => "shifter",
        }
    }
    pub fn from_id(s: &str) -> Option<Kind> {
        Kind::PALETTE.iter().copied().find(|k| k.id() == s)
    }
    pub fn is_mcu(self) -> bool {
        matches!(self, Kind::Uno | Kind::Esp32 | Kind::RasPi)
    }
    /// A part with an I2C target chip (something the MCU talks to).
    pub fn is_target(self) -> bool {
        matches!(
            self,
            Kind::Bme280 | Kind::Mpu6050 | Kind::Ssd1306 | Kind::Vl53l0x | Kind::Generic10k | Kind::BareChip
        )
    }
    /// Logic rail of an MCU board (pull-ups go to this voltage).
    pub fn logic_v(self) -> Option<f64> {
        match self {
            Kind::Uno => Some(5.0),
            Kind::Esp32 | Kind::RasPi => Some(3.3),
            _ => None,
        }
    }
    pub fn name(self, en: bool) -> &'static str {
        let (ko, eng) = match self {
            Kind::Uno => ("아두이노 우노 (5V)", "Arduino Uno (5 V)"),
            Kind::Esp32 => ("ESP32 (3.3V)", "ESP32 (3.3 V)"),
            Kind::RasPi => ("라즈베리파이 (3.3V)", "Raspberry Pi (3.3 V)"),
            Kind::Bme280 => ("BME280 모듈", "BME280 module"),
            Kind::Mpu6050 => ("MPU6050 모듈", "MPU6050 module"),
            Kind::Ssd1306 => ("SSD1306 OLED", "SSD1306 OLED"),
            Kind::Vl53l0x => ("VL53L0X 모듈", "VL53L0X module"),
            Kind::Generic10k => ("10k 풀업 달린 모듈", "Module with 10k pull-ups"),
            Kind::BareChip => ("맨 칩 (3.3V)", "Bare chip (3.3 V)"),
            Kind::PullUp => ("풀업 저항", "Pull-up resistor"),
            Kind::Wire => ("점퍼선", "Jumper wire"),
            Kind::LevelShifter => ("레벨 시프터", "Level shifter"),
        };
        if en {
            eng
        } else {
            ko
        }
    }
    /// Typical values for a hobby breakout of this kind (illustrative; editable).
    pub fn part(self) -> Part {
        let p = |r_kohm: f64, pin_pf: f64, addr: Option<u8>, five_v_ok: bool| Part {
            kind: self,
            r_kohm,
            pin_pf,
            addr,
            five_v_ok,
            len_cm: 0.0,
        };
        match self {
            Kind::Uno | Kind::Esp32 => p(0.0, 10.0, None, true),
            Kind::RasPi => p(1.8, 10.0, None, true),
            Kind::Bme280 => p(10.0, 10.0, Some(0x76), true),
            Kind::Mpu6050 => p(4.7, 10.0, Some(0x68), true),
            Kind::Ssd1306 => p(4.7, 10.0, Some(0x3C), true),
            Kind::Vl53l0x => p(10.0, 10.0, Some(0x29), true),
            Kind::Generic10k => p(10.0, 10.0, None, true),
            Kind::BareChip => p(0.0, 8.0, None, false),
            Kind::PullUp => p(4.7, 0.0, None, true),
            Kind::Wire => Part { len_cm: 10.0, ..p(0.0, 0.0, None, true) },
            Kind::LevelShifter => p(10.0, 10.0, None, true),
        }
    }
}

/// One part on the board. `r_kohm` is its pull-up per line (0 = none; for [`Kind::PullUp`] the
/// resistor value), `len_cm` is used by [`Kind::Wire`] only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub kind: Kind,
    pub r_kohm: f64,
    pub pin_pf: f64,
    pub addr: Option<u8>,
    pub five_v_ok: bool,
    pub len_cm: f64,
}

impl Part {
    pub fn with_r(self, r_kohm: f64) -> Part {
        Part { r_kohm, ..self }
    }
    pub fn with_len(self, len_cm: f64) -> Part {
        Part { len_cm, ..self }
    }
    pub fn pullup_ohm(&self) -> Option<f64> {
        (self.r_kohm > 0.0 && self.r_kohm.is_finite()).then_some(self.r_kohm * 1e3)
    }
}

/// Bus-level settings (edited in Details).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bus {
    pub speed: Speed,
    pub vol: f64,
    pub iol_ma: f64,
    /// Jumper capacitance per cm of SDA/SCL wire.
    pub pf_per_cm: f64,
    /// Breadboard strips, connectors and stray capacitance.
    pub stray_pf: f64,
}

impl Default for Bus {
    fn default() -> Bus {
        Bus { speed: Speed::Standard, vol: 0.4, iol_ma: 3.0, pf_per_cm: 1.0, stray_pf: 10.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub mcu: Option<Part>,
    pub parts: Vec<Part>,
    pub bus: Bus,
}

impl Default for Board {
    fn default() -> Board {
        Board { mcu: Some(Kind::Uno.part()), parts: vec![], bus: Bus::default() }
    }
}

impl Board {
    /// Place a part. An MCU replaces the current MCU; other parts are inserted at slot `at`
    /// (clamped) or appended. Returns false when the board is full.
    pub fn add(&mut self, p: Part, at: Option<usize>) -> bool {
        if p.kind.is_mcu() {
            self.mcu = Some(p);
            return true;
        }
        if self.parts.len() >= MAX_PARTS {
            return false;
        }
        let i = at.unwrap_or(self.parts.len()).min(self.parts.len());
        self.parts.insert(i, p);
        true
    }
    pub fn remove(&mut self, i: usize) {
        if i < self.parts.len() {
            self.parts.remove(i);
        }
    }
    fn all(&self) -> impl Iterator<Item = &Part> {
        self.mcu.iter().chain(self.parts.iter())
    }
    /// Voltage the pull-ups go to: the MCU logic rail (3.3 V without an MCU).
    pub fn logic_v(&self) -> f64 {
        self.mcu.and_then(|m| m.kind.logic_v()).unwrap_or(3.3)
    }
    pub fn pullups_ohm(&self) -> Vec<f64> {
        self.all().filter_map(Part::pullup_ohm).collect()
    }
    /// Combined pull-up per line (∞ when there is none).
    pub fn rp_eff_ohm(&self) -> f64 {
        parallel(&self.pullups_ohm())
    }
    pub fn wire_cm(&self) -> f64 {
        self.parts.iter().filter(|p| p.kind == Kind::Wire).map(|p| p.len_cm.max(0.0)).sum::<f64>()
            .abs()
    }
    pub fn pin_pf(&self) -> f64 {
        self.all().map(|p| p.pin_pf.max(0.0)).sum()
    }
    /// Chips with pins on the bus (MCU, modules, shifter).
    pub fn chip_count(&self) -> usize {
        self.all().filter(|p| !matches!(p.kind, Kind::PullUp | Kind::Wire)).count()
    }
    /// The equivalent calculator design, so Details shows the same numbers as the verdict.
    pub fn design(&self) -> Design {
        let n = self.chip_count().max(1) as f64;
        Design {
            vdd: self.logic_v(),
            speed: self.bus.speed,
            devices: n,
            pin_pf: self.pin_pf() / n,
            length_cm: self.wire_cm(),
            pf_per_cm: self.bus.pf_per_cm,
            extra_pf: self.bus.stray_pf,
            vol: self.bus.vol,
            iol_ma: self.bus.iol_ma,
            rp_kohm: self.rp_eff_ohm() / 1e3,
            n_extra: 0.0,
            r_extra_kohm: 10.0,
        }
    }
    pub fn verdict(&self) -> Verdict {
        verdict(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Works,
    Unstable,
    Fails,
}

impl Status {
    pub fn label(self, en: bool) -> &'static str {
        match (self, en) {
            (Status::Works, false) => "통신 됨",
            (Status::Unstable, false) => "불안정",
            (Status::Fails, false) => "안 됨",
            (Status::Works, true) => "Works",
            (Status::Unstable, true) => "Unstable",
            (Status::Fails, true) => "Fails",
        }
    }
}

/// One finding. Issues are produced in priority order; the first one is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Issue {
    NoMcu,
    NoTarget,
    /// 5 V pull-ups reach a 3.3 V-only part and there is no level shifter.
    LevelHazard { part: Kind },
    /// Two parts answer at the same address.
    AddressClash { addr: u8, kind: Kind },
    NoPullup { add: Option<f64> },
    /// Combined pull-up below Rp_min. `culprit` is the on-board pull-up whose removal alone fixes it.
    TooStrong { rp_eff: f64, sources: usize, culprit: Option<Kind>, pick: Option<f64> },
    TooMuchCap { cb_pf: f64 },
    /// Rise time over the limit (`unstable` = within 80–100 % of it). `add` = E12 resistor to add.
    TooWeak { rp_eff: f64, unstable: bool, add: Option<f64> },
    LongWire { cm: f64 },
}

impl Issue {
    pub fn status(&self) -> Status {
        match self {
            Issue::TooWeak { unstable: true, .. } | Issue::LongWire { .. } => Status::Unstable,
            _ => Status::Fails,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub status: Status,
    pub issues: Vec<Issue>,
    pub design: Design,
    pub report: Report,
}

impl Verdict {
    pub fn primary(&self) -> Option<&Issue> {
        self.issues.first()
    }
    /// (reason, fix) in plain words for the big verdict line.
    pub fn text(&self, en: bool) -> (String, String) {
        match self.primary() {
            Some(i) => issue_text(i, en),
            None => {
                let r = &self.report;
                let pct = (r.tr_ns / r.tr_max_ns * 100.0).round();
                if en {
                    (
                        format!("Pull-ups total {} and the signal rises in time ({pct}% of the limit).", fmt_ohm(r.rp_eff)),
                        "Nothing to fix — wire it up like this.".into(),
                    )
                } else {
                    (
                        format!("풀업 합성 {}, 신호가 제때 올라가요 (한계의 {pct}%).", fmt_ohm(r.rp_eff)),
                        "고칠 것 없어요 — 이대로 연결하면 돼요.".into(),
                    )
                }
            }
        }
    }
}

/// Compact resistance text: 915 Ω, 4.7 kΩ, 47 kΩ.
pub fn fmt_ohm(o: f64) -> String {
    if !o.is_finite() {
        return "∞".into();
    }
    let (v, u) = if o >= 1e6 {
        (o / 1e6, "MΩ")
    } else if o >= 1e3 {
        (o / 1e3, "kΩ")
    } else {
        (o, "Ω")
    };
    let s = if v >= 100.0 { format!("{v:.0}") } else if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    format!("{s}{u}")
}

fn addr_fix(k: Kind, en: bool) -> &'static str {
    match (k, en) {
        (Kind::Mpu6050, false) => "한 개의 AD0 핀을 VCC에 연결해 0x69로 바꾸세요.",
        (Kind::Mpu6050, true) => "Tie AD0 of one board to VCC to move it to 0x69.",
        (Kind::Bme280, false) => "한 개의 SDO 핀을 VCC에 연결해 0x77로 바꾸세요.",
        (Kind::Bme280, true) => "Tie SDO of one board to VCC to move it to 0x77.",
        (Kind::Ssd1306, false) => "한 개의 주소 점퍼를 바꿔 0x3D로 만드세요.",
        (Kind::Ssd1306, true) => "Move the address jumper on one display to 0x3D.",
        (Kind::Vl53l0x, false) => "XSHUT 핀으로 하나씩 켜면서 코드로 주소를 바꾸세요.",
        (Kind::Vl53l0x, true) => "Wake them one at a time with XSHUT and change the address in code.",
        (_, false) => "주소를 바꿀 수 있는지 확인하거나 I2C 멀티플렉서를 쓰세요.",
        (_, true) => "Check whether one address can be changed, or use an I2C multiplexer.",
    }
}

/// Plain-language (reason, fix) for one issue. No symbols like "Rp" — Details has those.
pub fn issue_text(i: &Issue, en: bool) -> (String, String) {
    let opt = |v: Option<f64>| v.map(fmt_ohm);
    match *i {
        Issue::NoMcu => if en {
            ("There is no MCU board, so nobody drives the bus.".into(), "Put an MCU board on the breadboard.".into())
        } else {
            ("MCU 보드가 없어서 버스를 움직일 주인이 없어요.".into(), "팔레트에서 MCU 보드를 올리세요.".into())
        },
        Issue::NoTarget => if en {
            ("There is no sensor or module for the MCU to talk to.".into(), "Add a sensor module from the palette.".into())
        } else {
            ("MCU가 말을 걸 센서나 모듈이 없어요.".into(), "팔레트에서 센서 모듈을 하나 올리세요.".into())
        },
        Issue::LevelHazard { part } => if en {
            (format!("The pull-ups go to 5 V but the {} only takes 3.3 V — it can be damaged.", part.name(true)),
             "Add a level shifter between them, or pull up to 3.3 V instead.".into())
        } else {
            (format!("풀업이 5V에 걸려 있는데 {}은(는) 3.3V 전용이에요 — 부품이 손상될 수 있어요.", part.name(false)),
             "사이에 레벨 시프터를 넣거나 풀업을 3.3V로 옮기세요.".into())
        },
        Issue::AddressClash { addr, kind } => if en {
            (format!("Two {} parts use the same address 0x{addr:02X}, so their answers collide.", kind.name(true)),
             addr_fix(kind, true).into())
        } else {
            (format!("{} 두 개가 같은 주소 0x{addr:02X}를 써서 대답이 부딪혀요.", kind.name(false)),
             addr_fix(kind, false).into())
        },
        Issue::NoPullup { add } => {
            let v = opt(add).unwrap_or_else(|| "4.7kΩ".into());
            if en {
                ("There is no pull-up at all — the lines can never go high.".into(),
                 format!("Add a {v} pull-up resistor (SDA and SCL to VCC)."))
            } else {
                ("풀업이 하나도 없어서 선이 HIGH로 올라가지 못해요.".into(),
                 format!("{v} 풀업 저항을 추가하세요 (SDA·SCL → VCC)."))
            }
        }
        Issue::TooStrong { rp_eff, sources, culprit, pick } => {
            let r = fmt_ohm(rp_eff);
            let reason = match (sources > 1, en) {
                (true, false) => format!("모듈 풀업이 겹쳐 합성 {r} — 센서가 LOW를 못 만들어요."),
                (true, true) => format!("Module pull-ups stack up to {r} — the sensor cannot pull the line low."),
                (false, false) => format!("풀업이 너무 강해요(합성 {r}) — 센서가 LOW를 못 만들어요."),
                (false, true) => format!("The pull-up is too strong ({r}) — the sensor cannot pull the line low."),
            };
            let fix = match (culprit, en) {
                (Some(k), false) => format!("{}의 풀업 저항을 떼세요.", k.name(false)),
                (Some(k), true) => format!("Remove the pull-up resistors on the {}.", k.name(true)),
                (None, false) if sources > 1 => "모듈 풀업을 하나만 남기고 떼세요.".into(),
                (None, true) if sources > 1 => "Keep the pull-ups on one board only and remove the rest.".into(),
                (None, false) => format!("{} 이상 값으로 바꾸세요.", opt(pick).unwrap_or_else(|| "2.2kΩ".into())),
                (None, true) => format!("Use {} or higher instead.", opt(pick).unwrap_or_else(|| "2.2kΩ".into())),
            };
            (reason, fix)
        }
        Issue::TooMuchCap { cb_pf } => if en {
            (format!("Too much wire and parts: the bus is {cb_pf:.0} pF, over the 400 pF limit."),
             "Use shorter wires or fewer parts (or a bus buffer).".into())
        } else {
            (format!("선과 부품이 너무 많아 버스 용량이 {cb_pf:.0} pF — 한계 400 pF를 넘어요."),
             "선을 짧게 하거나 부품을 줄이세요 (또는 버스 버퍼).".into())
        },
        Issue::TooWeak { rp_eff, unstable, add } => {
            let r = fmt_ohm(rp_eff);
            let reason = match (unstable, en) {
                (false, false) => format!("풀업이 너무 약해요(합성 {r}) — 신호가 제때 HIGH로 못 올라가요."),
                (false, true) => format!("The pull-up is too weak ({r}) — the signal cannot rise in time."),
                (true, false) => format!("풀업이 조금 약해요(합성 {r}) — 신호가 아슬아슬하게 올라가요."),
                (true, true) => format!("The pull-up is a bit weak ({r}) — the signal only just rises in time."),
            };
            let fix = match (add, en) {
                (Some(v), false) => format!("{} 풀업을 하나 추가하세요.", fmt_ohm(v)),
                (Some(v), true) => format!("Add one {} pull-up resistor.", fmt_ohm(v)),
                (None, false) => "속도를 낮추거나 선을 짧게 하세요.".into(),
                (None, true) => "Lower the bus speed or shorten the wires.".into(),
            };
            (reason, fix)
        }
        Issue::LongWire { cm } => if en {
            (format!("Jumpers add up to {cm:.0} cm — at this speed noise can corrupt bits now and then."),
             "Keep wires to 50 cm in total, or drop to 100 kHz.".into())
        } else {
            (format!("선 길이 합계 {cm:.0}cm — 이 속도에선 잡음으로 가끔 비트가 깨질 수 있어요."),
             "선 합계를 50cm 이하로 줄이거나 100kHz로 낮추세요.".into())
        },
    }
}

/// E12 resistor to put in parallel with `rp_eff` so the total lands in [rp_min, 0.8·rp_max].
pub fn add_pick(rp_eff: f64, rp_min: f64, rp_max: f64) -> Option<f64> {
    let g_eff = if rp_eff.is_finite() && rp_eff > 0.0 { 1.0 / rp_eff } else { 0.0 };
    let g_need_lo = 1.0 / (0.8 * rp_max) - g_eff;
    let g_need_hi = 1.0 / rp_min - g_eff;
    if g_need_hi.is_nan() || g_need_hi <= 0.0 {
        return None;
    }
    let hi = if g_need_lo > 0.0 { 1.0 / g_need_lo } else { 1e7 };
    e12_pick(1.0 / g_need_hi, hi)
}

/// Parse an I2C address like "0x68", "68" (hex) or empty (= none). Valid 7-bit range 0x03–0x77.
pub fn parse_addr(s: &str) -> Result<Option<u8>, NumErr> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let h = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let v = u8::from_str_radix(h, 16).map_err(|_| NumErr::NotNumber)?;
    if (0x03..=0x77).contains(&v) {
        Ok(Some(v))
    } else {
        Err(NumErr::OutOfRange(3.0, 119.0))
    }
}

pub fn verdict(b: &Board) -> Verdict {
    let d = b.design();
    let r = analyze(&d);
    let mut issues = vec![];
    if b.mcu.is_none() {
        issues.push(Issue::NoMcu);
    }
    if !b.parts.iter().any(|p| p.kind.is_target()) {
        issues.push(Issue::NoTarget);
    }
    let pullups = b.pullups_ohm();
    let shifter = b.parts.iter().any(|p| p.kind == Kind::LevelShifter);
    if b.logic_v() > 3.6 && !pullups.is_empty() && !shifter {
        if let Some(p) = b.parts.iter().find(|p| p.kind.is_target() && !p.five_v_ok) {
            issues.push(Issue::LevelHazard { part: p.kind });
        }
    }
    let addrs: Vec<(u8, Kind)> = b.parts.iter().filter_map(|p| p.addr.map(|a| (a, p.kind))).collect();
    if let Some((a, k)) = addrs.iter().enumerate().find_map(|(i, x)| addrs[..i].iter().any(|y| y.0 == x.0).then_some(*x)) {
        issues.push(Issue::AddressClash { addr: a, kind: k });
    }
    if pullups.is_empty() {
        issues.push(Issue::NoPullup { add: add_pick(f64::INFINITY, r.rp_min, r.rp_max) });
    } else {
        if r.rp_eff < r.rp_min {
            let onboard: Vec<&Part> = b.all().filter(|p| p.kind != Kind::PullUp && p.pullup_ohm().is_some()).collect();
            let culprit = onboard
                .iter()
                .min_by(|x, y| x.r_kohm.total_cmp(&y.r_kohm))
                .filter(|c| {
                    let rest: Vec<f64> = b.all().filter(|p| !std::ptr::eq(*p, **c)).filter_map(Part::pullup_ohm).collect();
                    parallel(&rest) >= r.rp_min
                })
                .map(|c| c.kind);
            issues.push(Issue::TooStrong { rp_eff: r.rp_eff, sources: pullups.len(), culprit, pick: e12_pick(r.rp_min, r.rp_max) });
        }
        if r.cb_pf > CB_FAIL_PF {
            issues.push(Issue::TooMuchCap { cb_pf: r.cb_pf });
        }
        let lvl = grade(r.tr_ns, r.tr_max_ns);
        if lvl != Level::Pass && r.rp_eff >= r.rp_min {
            issues.push(Issue::TooWeak { rp_eff: r.rp_eff, unstable: lvl == Level::Warn, add: add_pick(r.rp_eff, r.rp_min, r.rp_max) });
        }
    }
    if b.wire_cm() > WIRE_WARN_CM && b.bus.speed != Speed::Standard {
        issues.push(Issue::LongWire { cm: b.wire_cm() });
    }
    // Stable sort: hard failures first, warnings after, priority kept within each group.
    issues.sort_by_key(|i| std::cmp::Reverse(i.status()));
    let status = issues.iter().map(Issue::status).max().unwrap_or(Status::Works);
    Verdict { status, issues, design: d, report: r }
}

/// The four starter boards. Each name carries the verdict it produces (checked by tests).
pub fn presets() -> Vec<(&'static str, &'static str, Board)> {
    let bus = |speed| Bus { speed, ..Bus::default() };
    let wire = |cm| Kind::Wire.part().with_len(cm);
    vec![
        (
            "아두이노 우노 + BME280 하나 (됨)",
            "Arduino Uno + one BME280 (works)",
            Board { mcu: Some(Kind::Uno.part()), parts: vec![wire(20.0), Kind::Bme280.part()], bus: bus(Speed::Standard) },
        ),
        (
            "ESP32 + 모듈 4개 풀업 겹침 (안 됨)",
            "ESP32 + 4 modules, stacked pull-ups (fails)",
            Board {
                mcu: Some(Kind::Esp32.part()),
                parts: vec![
                    wire(10.0),
                    Kind::Mpu6050.part().with_r(2.2),
                    Kind::Ssd1306.part(),
                    Kind::Bme280.part().with_r(4.7),
                    Kind::Vl53l0x.part().with_r(4.7),
                ],
                bus: bus(Speed::Fast),
            },
        ),
        (
            "라즈베리파이 + 50cm 선 + OLED 400kHz (불안정)",
            "Raspberry Pi + 50 cm wire + OLED at 400 kHz (unstable)",
            Board {
                mcu: Some(Kind::RasPi.part()),
                parts: vec![wire(10.0), wire(50.0), Kind::Ssd1306.part()],
                bus: bus(Speed::Fast),
            },
        ),
        (
            "5V 풀업 + 3.3V 센서 (손상 위험)",
            "5 V pull-up + 3.3 V sensor (damage risk)",
            Board {
                mcu: Some(Kind::Uno.part()),
                parts: vec![wire(10.0), Kind::PullUp.part().with_r(4.7), Kind::BareChip.part()],
                bus: bus(Speed::Standard),
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(mcu: Kind, parts: Vec<Part>, speed: Speed) -> Board {
        Board { mcu: Some(mcu.part()), parts, bus: Bus { speed, ..Bus::default() } }
    }
    fn first(b: &Board) -> Option<Issue> {
        b.verdict().primary().copied()
    }

    #[test]
    fn presets_match_their_names() {
        let p = presets();
        let st: Vec<Status> = p.iter().map(|(_, _, b)| b.verdict().status).collect();
        assert_eq!(st, [Status::Works, Status::Fails, Status::Unstable, Status::Fails]);
        assert!(matches!(first(&p[1].2), Some(Issue::TooStrong { culprit: Some(Kind::Mpu6050), .. })));
        assert!(matches!(first(&p[2].2), Some(Issue::LongWire { .. })));
        assert_eq!(first(&p[3].2), Some(Issue::LevelHazard { part: Kind::BareChip }));
        for (ko, en, b) in &p {
            let v = b.verdict();
            assert!(ko.contains(match v.status { Status::Works => "됨", Status::Unstable => "불안정", Status::Fails => "안 됨" })
                || ko.contains("손상"), "{ko}");
            assert!(!en.is_empty());
        }
    }

    #[test]
    fn no_pullup_fails_and_suggests_value() {
        let b = board(Kind::Esp32, vec![Kind::BareChip.part()], Speed::Fast);
        let v = b.verdict();
        assert_eq!(v.status, Status::Fails);
        match v.primary() {
            Some(Issue::NoPullup { add: Some(r) }) => assert!(*r >= v.report.rp_min && *r <= v.report.rp_max * 0.8),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stacked_pullups_below_rp_min_fail() {
        // 2.2k ∥ 4.7k ∥ 4.7k ∥ 4.7k = 915 Ω < 966.7 Ω (3.3 V, 0.4 V, 3 mA).
        let b = &presets()[1].2;
        assert!((b.rp_eff_ohm() - 915.0).abs() < 1.0, "{}", b.rp_eff_ohm());
        let v = b.verdict();
        assert!(v.report.rp_eff < v.report.rp_min);
        let (reason, fix) = v.text(false);
        assert!(reason.contains("겹쳐") && reason.contains("915Ω"), "{reason}");
        assert!(fix.contains("MPU6050"), "{fix}");
    }

    #[test]
    fn weak_pullup_rise_time_fails_with_add_fix() {
        // 47 kΩ, Cb = 10 + 10 + 20 + 10 = 50 pF at 400 kHz → tr ≈ 1991 ns > 300 ns.
        let b = board(Kind::Esp32, vec![Kind::Wire.part().with_len(20.0), Kind::Generic10k.part().with_r(47.0)], Speed::Fast);
        let v = b.verdict();
        assert_eq!(v.status, Status::Fails);
        let Some(Issue::TooWeak { unstable: false, add: Some(add), .. }) = v.primary().copied() else { panic!("{:?}", v.issues) };
        let mut fixed = b.clone();
        fixed.add(Kind::PullUp.part().with_r(add / 1e3), None);
        assert_eq!(fixed.verdict().status, Status::Works);
        assert!(v.text(false).0.contains("47kΩ"));
    }

    #[test]
    fn rise_time_80_to_100_percent_is_unstable() {
        // 10 kΩ at 400 kHz with Cb = 10 + 10 + 10 + 3·1 → tr = 0.8473·10k·33 pF ≈ 280 ns (93 %).
        let b = board(Kind::Esp32, vec![Kind::Wire.part().with_len(3.0), Kind::Bme280.part()], Speed::Fast);
        let v = b.verdict();
        let pct = v.report.tr_ns / v.report.tr_max_ns;
        assert!(pct > 0.8 && pct <= 1.0, "{pct}");
        assert_eq!(v.status, Status::Unstable);
        assert!(matches!(v.primary(), Some(Issue::TooWeak { unstable: true, .. })));
    }

    #[test]
    fn capacitance_over_400pf_fails() {
        let mut b = board(Kind::Esp32, vec![Kind::Bme280.part().with_r(1.0)], Speed::Standard);
        b.bus.stray_pf = 420.0;
        let v = b.verdict();
        assert_eq!(v.status, Status::Fails);
        assert!(v.issues.iter().any(|i| matches!(i, Issue::TooMuchCap { .. })), "{:?}", v.issues);
    }

    #[test]
    fn five_volt_pullup_on_3v3_part_needs_shifter() {
        let mut b = presets()[3].2.clone();
        assert_eq!(b.verdict().status, Status::Fails);
        assert!(b.verdict().text(false).0.contains("손상"));
        b.add(Kind::LevelShifter.part(), Some(1));
        assert!(!b.verdict().issues.iter().any(|i| matches!(i, Issue::LevelHazard { .. })));
        // A 3.3 V MCU never puts 5 V on the bus.
        let ok = board(Kind::Esp32, vec![Kind::PullUp.part(), Kind::BareChip.part()], Speed::Standard);
        assert_eq!(ok.verdict().status, Status::Works);
    }

    #[test]
    fn duplicate_addresses_clash() {
        let two = |k: Kind| board(Kind::Esp32, vec![k.part(), k.part()], Speed::Standard);
        assert_eq!(first(&two(Kind::Mpu6050)), Some(Issue::AddressClash { addr: 0x68, kind: Kind::Mpu6050 }));
        assert_eq!(first(&two(Kind::Vl53l0x)), Some(Issue::AddressClash { addr: 0x29, kind: Kind::Vl53l0x }));
        let mut b = two(Kind::Mpu6050);
        b.parts[1].addr = parse_addr("0x69").unwrap_or(None);
        assert!(!b.verdict().issues.iter().any(|i| matches!(i, Issue::AddressClash { .. })));
        assert_eq!(b.verdict().status, Status::Works);
    }

    #[test]
    fn long_wire_only_matters_at_400k() {
        let p = &presets()[2].2;
        assert_eq!(p.wire_cm(), 60.0);
        let mut slow = p.clone();
        slow.bus.speed = Speed::Standard;
        assert_eq!(slow.verdict().status, Status::Works);
        let mut fifty = p.clone();
        fifty.remove(0);
        assert_eq!(fifty.wire_cm(), 50.0);
        assert_eq!(fifty.verdict().status, Status::Works, "exactly 50 cm is not over 50 cm");
    }

    #[test]
    fn verdict_words_are_plain() {
        let mut boards: Vec<Board> = presets().into_iter().map(|p| p.2).collect();
        boards.push(Board { mcu: None, parts: vec![], bus: Bus::default() });
        boards.push(board(Kind::Esp32, vec![Kind::Generic10k.part().with_r(47.0)], Speed::Fast));
        for b in &boards {
            for en in [false, true] {
                let (r, f) = b.verdict().text(en);
                assert!(!r.contains("Rp") && !f.contains("Rp") && !r.contains("NaN") && !r.is_empty(), "{r} / {f}");
            }
        }
        let empty = boards[4].verdict();
        assert_eq!(empty.primary(), Some(&Issue::NoMcu));
    }

    #[test]
    fn board_maps_to_design() {
        let b = &presets()[0].2;
        let d = b.design();
        assert_eq!(d.vdd, 5.0);
        assert!((d.cb_pf() - 50.0).abs() < 1e-9, "{}", d.cb_pf());
        assert!((d.rp_eff_ohm() - 10_000.0).abs() < 1e-6);
        let mut full = Board::default();
        for _ in 0..MAX_PARTS {
            assert!(full.add(Kind::Wire.part(), None));
        }
        assert!(!full.add(Kind::Wire.part(), None));
        assert!(full.add(Kind::Esp32.part(), None) && full.logic_v() == 3.3);
    }

    #[test]
    fn address_parsing() {
        assert_eq!(parse_addr(" 0x3c "), Ok(Some(0x3C)));
        assert_eq!(parse_addr("68"), Ok(Some(0x68)));
        assert_eq!(parse_addr(""), Ok(None));
        assert_eq!(parse_addr("zz"), Err(NumErr::NotNumber));
        assert_eq!(parse_addr("0x80"), Err(NumErr::OutOfRange(3.0, 119.0)));
        assert_eq!(fmt_ohm(915.2), "915Ω");
        assert_eq!(fmt_ohm(4700.0), "4.7kΩ");
        assert_eq!(fmt_ohm(47_000.0), "47kΩ");
    }
}
