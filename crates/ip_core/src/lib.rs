//! I2C pull-up resistor and rise-time planning model.
//!
//! Pure Rust, no I/O. Formulas follow the usual I2C-bus specification estimates:
//! `Rp_min = (VDD - VOL) / IOL`, `tr = 0.8473 * Rp * Cb` (0.3·VDD → 0.7·VDD),
//! `Rp_max = tr_max / (0.8473 * Cb)`. Planning estimates only.

pub mod board;
pub use board::*;

/// ln(0.7 / 0.3): RC time constants between the 30 % and 70 % thresholds.
pub const K_RISE: f64 = 0.8473;
/// Fixed fall-time estimate used for f_max (a strong open-drain driver is fast).
pub const TF_EST_NS: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Standard,
    Fast,
    FastPlus,
}

impl Speed {
    pub fn from_index(i: usize) -> Speed {
        match i {
            0 => Speed::Standard,
            2 => Speed::FastPlus,
            _ => Speed::Fast,
        }
    }
    fn pick(self, v: [f64; 3]) -> f64 {
        v[self as usize]
    }
    pub fn clock_hz(self) -> f64 {
        self.pick([100e3, 400e3, 1e6])
    }
    pub fn tr_max_ns(self) -> f64 {
        self.pick([1000.0, 300.0, 120.0])
    }
    pub fn t_high_min_ns(self) -> f64 {
        self.pick([4000.0, 600.0, 260.0])
    }
    pub fn t_low_min_ns(self) -> f64 {
        self.pick([4700.0, 1300.0, 500.0])
    }
    pub fn cb_limit_pf(self) -> f64 {
        self.pick([400.0, 400.0, 550.0])
    }
    pub fn default_iol_ma(self) -> f64 {
        self.pick([3.0, 3.0, 20.0])
    }
}

/// Numeric input fields with their DOM id, allowed range and integer flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Vdd,
    Devices,
    PinPf,
    LengthCm,
    PfPerCm,
    ExtraPf,
    Vol,
    Iol,
    Rp,
    NExtra,
    RExtra,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Vdd,
        Field::Devices,
        Field::PinPf,
        Field::LengthCm,
        Field::PfPerCm,
        Field::ExtraPf,
        Field::Vol,
        Field::Iol,
        Field::Rp,
        Field::NExtra,
        Field::RExtra,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Field::Vdd => "vdd",
            Field::Devices => "devices",
            Field::PinPf => "pin_pf",
            Field::LengthCm => "length_cm",
            Field::PfPerCm => "pf_per_cm",
            Field::ExtraPf => "extra_pf",
            Field::Vol => "vol",
            Field::Iol => "iol",
            Field::Rp => "rp",
            Field::NExtra => "n_extra",
            Field::RExtra => "r_extra",
        }
    }
    /// Inclusive (min, max) in the field's display unit.
    pub fn range(self) -> (f64, f64) {
        match self {
            Field::Vdd => (0.8, 15.0),
            Field::Devices => (1.0, 127.0),
            Field::PinPf => (0.0, 100.0),
            Field::LengthCm => (0.0, 10_000.0),
            Field::PfPerCm => (0.0, 10.0),
            Field::ExtraPf => (0.0, 5_000.0),
            Field::Vol => (0.0, 2.0),
            Field::Iol => (0.1, 100.0),
            Field::Rp | Field::RExtra => (0.1, 1_000.0),
            Field::NExtra => (0.0, 16.0),
        }
    }
    pub fn integer(self) -> bool {
        matches!(self, Field::Devices | Field::NExtra)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumErr {
    Empty,
    NotNumber,
    NotInteger,
    OutOfRange(f64, f64),
}

/// Parse user text for a field. Never panics; rejects NaN/inf, text, out-of-range values.
pub fn parse_field(f: Field, s: &str) -> Result<f64, NumErr> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() {
        return Err(NumErr::Empty);
    }
    let v: f64 = s.parse().map_err(|_| NumErr::NotNumber)?;
    if !v.is_finite() {
        return Err(NumErr::NotNumber);
    }
    let (lo, hi) = f.range();
    if v < lo || v > hi {
        return Err(NumErr::OutOfRange(lo, hi));
    }
    if f.integer() && v.fract() != 0.0 {
        return Err(NumErr::NotInteger);
    }
    Ok(v)
}

/// Cross-field problems that make the model meaningless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignErr {
    VolNotBelowVdd,
    ZeroCapacitance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Design {
    pub vdd: f64,
    pub speed: Speed,
    pub devices: f64,
    pub pin_pf: f64,
    pub length_cm: f64,
    pub pf_per_cm: f64,
    pub extra_pf: f64,
    pub vol: f64,
    pub iol_ma: f64,
    /// Main pull-up in kΩ.
    pub rp_kohm: f64,
    /// Extra module pull-ups in parallel and their value in kΩ.
    pub n_extra: f64,
    pub r_extra_kohm: f64,
}

impl Design {
    pub fn base_cb_pf(&self) -> f64 {
        self.devices * self.pin_pf + self.length_cm * self.pf_per_cm
    }
    pub fn cb_pf(&self) -> f64 {
        self.base_cb_pf() + self.extra_pf
    }
    pub fn rp_eff_ohm(&self) -> f64 {
        let n = self.n_extra.max(0.0) as usize;
        let mut rs = vec![self.rp_kohm * 1e3];
        rs.extend(std::iter::repeat_n(self.r_extra_kohm * 1e3, n));
        parallel(&rs)
    }
    pub fn check(&self) -> Result<(), DesignErr> {
        if self.vol >= self.vdd {
            return Err(DesignErr::VolNotBelowVdd);
        }
        if self.cb_pf() <= 0.0 {
            return Err(DesignErr::ZeroCapacitance);
        }
        Ok(())
    }
}

/// Parallel combination of resistors (Ω). Non-positive entries are ignored.
pub fn parallel(rs: &[f64]) -> f64 {
    let g: f64 = rs.iter().filter(|r| **r > 0.0).map(|r| 1.0 / r).sum();
    if g > 0.0 {
        1.0 / g
    } else {
        f64::INFINITY
    }
}

/// Main-board resistor that, in parallel with `n` resistors of `rx` Ω, gives `target` Ω.
pub fn solve_main_rp(target: f64, n: f64, rx: f64) -> Option<f64> {
    let g = 1.0 / target - n.max(0.0) / rx;
    (g > 0.0 && g.is_finite()).then(|| 1.0 / g)
}

pub fn rp_min_ohm(vdd: f64, vol: f64, iol_ma: f64) -> f64 {
    (vdd - vol) / (iol_ma * 1e-3)
}

pub fn rp_max_ohm(tr_max_ns: f64, cb_pf: f64) -> f64 {
    tr_max_ns * 1e-9 / (K_RISE * cb_pf * 1e-12)
}

pub fn rise_time_ns(rp_ohm: f64, cb_pf: f64) -> f64 {
    K_RISE * rp_ohm * cb_pf * 1e-3
}

const E12: [f64; 12] = [1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2];

/// E12 value (10 Ω … 8.2 MΩ) inside [lo, hi] closest (log scale) to the geometric middle.
pub fn e12_pick(lo: f64, hi: f64) -> Option<f64> {
    if !(lo > 0.0 && lo <= hi && hi.is_finite()) {
        return None;
    }
    let mid = (lo * hi).sqrt();
    (1..=6)
        .flat_map(|d| E12.iter().map(move |m| m * 10f64.powi(d)))
        .filter(|v| *v >= lo * (1.0 - 1e-9) && *v <= hi * (1.0 + 1e-9))
        .min_by(|a, b| (a / mid).ln().abs().total_cmp(&(b / mid).ln().abs()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Pass,
    Warn,
    Fail,
}

/// Pass below 80 % of the limit, warn up to the limit, fail above.
pub fn grade(value: f64, limit: f64) -> Level {
    if value <= 0.8 * limit {
        Level::Pass
    } else if value <= limit {
        Level::Warn
    } else {
        Level::Fail
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    pub cb_pf: f64,
    pub cb_limit_pf: f64,
    pub rp_min: f64,
    pub rp_max: f64,
    pub feasible: bool,
    pub e12: Option<f64>,
    pub rp_eff: f64,
    pub tr_ns: f64,
    pub tr_max_ns: f64,
    pub margin_pct: f64,
    pub sink_ma: f64,
    pub f_max_hz: f64,
    pub power_mw: f64,
    pub reaches_vih: bool,
    pub tr_level: Level,
    pub sink_level: Level,
    pub cb_level: Level,
}

pub fn analyze(d: &Design) -> Report {
    let s = d.speed;
    let cb = d.cb_pf();
    let rp_min = rp_min_ohm(d.vdd, d.vol, d.iol_ma);
    let rp_max = rp_max_ohm(s.tr_max_ns(), cb);
    let rp_eff = d.rp_eff_ohm();
    let tr = rise_time_ns(rp_eff, cb);
    let sink = (d.vdd - d.vol) / rp_eff * 1e3;
    let tau = rp_eff * cb * 1e-3;
    let t_vih = tau * ((d.vdd - d.vol) / (0.3 * d.vdd)).ln().max(0.0);
    let t_period = 1e9 / s.clock_hz();
    Report {
        cb_pf: cb,
        cb_limit_pf: s.cb_limit_pf(),
        rp_min,
        rp_max,
        feasible: rp_min <= rp_max,
        e12: e12_pick(rp_min, rp_max),
        rp_eff,
        tr_ns: tr,
        tr_max_ns: s.tr_max_ns(),
        margin_pct: (s.tr_max_ns() - tr) / s.tr_max_ns() * 100.0,
        sink_ma: sink,
        f_max_hz: 1e9 / (s.t_low_min_ns() + s.t_high_min_ns() + tr + TF_EST_NS),
        power_mw: d.vdd * d.vdd / rp_eff * 1e3,
        reaches_vih: t_vih <= t_period / 2.0,
        tr_level: grade(tr, s.tr_max_ns()),
        sink_level: grade(sink, d.iol_ma),
        cb_level: grade(cb, s.cb_limit_pf()),
    }
}

/// Line voltage at `t_ns` for a window of 1.25 clock periods: low, released (RC rise), driven low.
pub fn wave_v(d: &Design, t_ns: f64) -> f64 {
    let p = 1e9 / d.speed.clock_hz();
    let tau_r = (d.rp_eff_ohm() * d.cb_pf() * 1e-3).max(1e-6);
    let tau_f = TF_EST_NS / K_RISE;
    let rise = |t: f64| d.vdd - (d.vdd - d.vol) * (-(t - p / 2.0) / tau_r).exp();
    if t_ns < p / 2.0 {
        d.vol
    } else if t_ns < p {
        rise(t_ns)
    } else {
        d.vol + (rise(p) - d.vol) * (-(t_ns - p) / tau_f).exp()
    }
}

/// RFC 4180-style CSV: quote fields containing comma, quote or newline.
pub fn to_csv(rows: &[Vec<String>]) -> String {
    let esc = |f: &String| {
        if f.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", f.replace('"', "\"\""))
        } else {
            f.clone()
        }
    };
    rows.iter()
        .map(|r| r.iter().map(esc).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn rp_min_3v3() {
        assert!(close(rp_min_ohm(3.3, 0.4, 3.0), 966.7, 0.05));
    }

    #[test]
    fn rp_max_fast_200pf() {
        assert!(close(rp_max_ohm(Speed::Fast.tr_max_ns(), 200.0), 1770.0, 0.5));
    }

    #[test]
    fn rise_time_2k2_100pf() {
        assert!(close(rise_time_ns(2200.0, 100.0), 186.4, 0.05));
    }

    #[test]
    fn parallel_pullups() {
        let r = parallel(&[4700.0, 10_000.0, 10_000.0]);
        assert!(close(r, 2422.7, 0.1), "{r}");
        let main = solve_main_rp(r, 2.0, 10_000.0).unwrap_or(0.0);
        assert!(close(main, 4700.0, 0.1));
    }

    #[test]
    fn e12_inside_window() {
        // 966.7 Ω … 4721 Ω, geometric middle ≈ 2136 Ω → 2.2 kΩ.
        let v = e12_pick(966.7, 4721.0);
        assert_eq!(v, Some(2200.0));
        assert_eq!(e12_pick(1000.0, 1100.0), Some(1000.0));
    }

    #[test]
    fn infeasible_window() {
        let d = Design {
            vdd: 3.3,
            speed: Speed::FastPlus,
            devices: 20.0,
            pin_pf: 10.0,
            length_cm: 400.0,
            pf_per_cm: 1.0,
            extra_pf: 0.0,
            vol: 0.4,
            iol_ma: 3.0,
            rp_kohm: 2.2,
            n_extra: 0.0,
            r_extra_kohm: 10.0,
        };
        let r = analyze(&d);
        assert!(!r.feasible && r.rp_min > r.rp_max && r.e12.is_none());
        assert_eq!(r.cb_level, Level::Fail);
    }

    #[test]
    fn csv_escaping() {
        let rows = vec![vec!["a,b".to_string(), "say \"hi\"".into(), "plain".into()]];
        assert_eq!(to_csv(&rows), "\"a,b\",\"say \"\"hi\"\"\",plain\r\n");
    }

    #[test]
    fn invalid_inputs_never_panic() {
        assert_eq!(parse_field(Field::Vdd, ""), Err(NumErr::Empty));
        assert_eq!(parse_field(Field::Vdd, "abc"), Err(NumErr::NotNumber));
        assert_eq!(parse_field(Field::Vdd, "inf"), Err(NumErr::NotNumber));
        assert_eq!(parse_field(Field::Vdd, "0"), Err(NumErr::OutOfRange(0.8, 15.0)));
        assert_eq!(parse_field(Field::Rp, "-1"), Err(NumErr::OutOfRange(0.1, 1000.0)));
        assert_eq!(parse_field(Field::Rp, "1e12"), Err(NumErr::OutOfRange(0.1, 1000.0)));
        assert_eq!(parse_field(Field::Devices, "2.5"), Err(NumErr::NotInteger));
        assert_eq!(parse_field(Field::Vdd, " 3,3 "), Ok(3.3));
    }
}
