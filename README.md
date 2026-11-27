# I2C Pull

I2C 풀업 저항 & 버스 상승 시간 플래너 — Rust → WebAssembly, 100% 브라우저 내 실행.
I2C pull-up resistor & bus rise-time planner — Rust → WebAssembly, runs 100% in the browser.

[한국어](#한국어) · [English](#english)

---

## 한국어

### 무엇을 하나요
"이 전원 전압, 이 센서 개수, 이 케이블 길이에서 어떤 풀업 저항이 I2C 상승 시간과 싱크 전류 규격을 지키고,
버스를 얼마나 빠르게 돌릴 수 있을까?"에 답하는 작은 정적 웹 앱입니다.

- 버스 용량 계산기: 디바이스 수 × 핀 용량 + 배선 길이 × pF/cm + 추가 고정 용량 → 총 Cb와 규격 한계(Sm/Fm 400 pF, Fm+ 550 pF)
- Rp_min / Rp_max, 사용 가능한 저항 범위, 범위 안의 추천 E12 값(기하 평균에 가장 가까운 값)
- 선택한 Rp(모듈 풀업 병렬 포함)의 상승 시간, 여유율, 싱크 전류, 추정 최대 클럭, 정적 전력
- 통과/주의/실패 칩: 상승 시간, 싱크 전류, Cb 한계
- 히어로 시각화 (Canvas 2D, 외부 차트 라이브러리 없음)
  1. SCL/SDA 파형: 한 클럭 주기의 RC 상승과 빠른 하강, VIH/VIL 점선, tr 구간 음영, 이상적 구형파.
     다음 LOW 구간 전에 VIH에 도달하지 못하면 빨간색으로 표시
  2. Rp–Cb 허용 영역 그래프: 점을 드래그하면 Rp와 추가 pF 값이 바뀝니다
- 비교 모드: 현재 설계를 회색 고스트로 고정하고 Δ tr, Δ f_max 표시
- 예시용 프리셋 3개(수정 가능)와 결과표 CSV 내보내기

### 모델 / 수식 (I2C-bus specification의 표준 추정식)
| 항목 | 식 |
|---|---|
| 최소 풀업 | Rp_min = (VDD − VOL) / IOL |
| 상승 시간 (0.3·VDD → 0.7·VDD) | tr = 0.8473 · Rp · Cb |
| 최대 풀업 | Rp_max = tr_max / (0.8473 · Cb), tr_max = 1000 ns (Sm), 300 ns (Fm), 120 ns (Fm+) |
| 최대 클럭 추정 | f_max ≈ 1 / (tLOW_min + tHIGH_min + tr + tf), tLOW/tHIGH = 4.7/4.0 µs (Sm), 1.3/0.6 µs (Fm), 0.5/0.26 µs (Fm+), tf = 20 ns 고정 가정 |
| 정적 전력 (라인당, LOW 유지) | P = VDD² / Rp |
| 병렬 풀업 | 1/R_eff = 1/Rp + n/R_module |

판정: 한계의 80 % 이하 = 통과, 한계 이하 = 주의, 초과 = 실패.
Rp_min > Rp_max이면 어떤 저항도 맞지 않는다고 명확히 표시합니다 → 용량 줄이기, 속도 낮추기, 버스 버퍼/액티브 풀업 사용.

### 한계 (정직한 안내)
계획용 추정치일 뿐입니다. 직렬 저항, 누설 전류, 개별 부품의 입력 임계값, 링잉, 크로스토크, 레벨 시프터는 고려하지 않습니다.
실제 상승 시간은 반드시 오실로스코프로 측정하세요.

### 조작법
- 왼쪽(모바일은 하단 시트) 설정 카드에 값을 입력합니다. 카드는 접을 수 있습니다.
- Rp 슬라이더는 로그 스케일이며 허용 범위 주변을 덮습니다.
- Rp–Cb 그래프의 점 드래그 = Rp와 추가 pF 변경. 포커스 후 ↑/↓ = Rp, ←/→ = 용량 ±5 pF.
- 파형에 마우스를 올리거나 포커스 후 ←/→ 로 시간·전압 툴팁 확인.
- 잘못된 입력(0, 음수, 빈 값, 문자, 1e12 등)은 패닉 없이 입력란 아래에 메시지를 보여줍니다.

### 로컬 빌드
```
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.22.2
cargo test
trunk build --release      # 결과물: dist/ (/i2c-pull/ 하위 경로용)
trunk serve                # 개발 서버
bash tools/privacy_check.sh
```

### 프라이버시
분석 도구, 트래커, 외부 폰트/CDN, 텔레메트리, 서버 구성요소, 런타임 네트워크 요청이 없습니다.
입력값은 브라우저 밖으로 나가지 않으며 저장되지도 않습니다. CSV는 브라우저 안에서 생성됩니다.

### 라이선스
MIT — `LICENSE` 참고. Copyright i2c-pull contributors.

---

## English

### What it does
A small static web app that answers: "With this supply, this many sensors and this cable length, which pull-up
resistor keeps my I2C bus inside the spec rise time and sink current, and how fast can it run?"

- Bus capacitance builder: devices × pin capacitance + trace length × pF/cm + extra fixed pF → total Cb and the
  spec limit (400 pF for Sm/Fm, 550 pF for Fm+)
- Rp_min / Rp_max, the feasible window, and the recommended E12 value inside it (closest to the geometric middle)
- Rise time, margin, sink current, estimated max clock and static power of the chosen Rp (including parallel
  module pull-ups)
- Pass/warn/fail chips for rise time, sink current and the Cb limit
- Hero visualisation (Canvas 2D, no chart library)
  1. SCL/SDA waveform: one clock period with RC rise and fast fall, dashed VIH/VIL, shaded tr window and the ideal
     square wave behind it. Coloured red if the rise does not reach VIH before the next low phase
  2. Rp–Cb window plot: drag the design dot to change Rp and the extra-pF field
- Compare mode: pin the current design as a grey ghost and see Δ tr and Δ f_max
- Three illustrative, editable presets and CSV export of the result table

### Model / formulas (standard I2C-bus specification estimates)
| Quantity | Formula |
|---|---|
| Minimum pull-up | Rp_min = (VDD − VOL) / IOL |
| Rise time (0.3·VDD → 0.7·VDD) | tr = 0.8473 · Rp · Cb |
| Maximum pull-up | Rp_max = tr_max / (0.8473 · Cb), tr_max = 1000 ns (Sm), 300 ns (Fm), 120 ns (Fm+) |
| Max clock estimate | f_max ≈ 1 / (tLOW_min + tHIGH_min + tr + tf), tLOW/tHIGH = 4.7/4.0 µs (Sm), 1.3/0.6 µs (Fm), 0.5/0.26 µs (Fm+), tf fixed at 20 ns |
| Static power (per line, held low) | P = VDD² / Rp |
| Parallel pull-ups | 1/R_eff = 1/Rp + n/R_module |

Grading: ≤ 80 % of the limit = pass, ≤ limit = warn, above = fail.
If Rp_min > Rp_max the app says plainly that no resistor works → reduce capacitance, lower the speed, or use a bus
buffer / active pull-up.

### Limitations (honesty note)
Planning estimate only. Ignores series resistors, leakage, input thresholds of specific parts, ringing, crosstalk
and level shifters. Always measure the real rise time with a scope.

### Controls
- Enter values in the settings cards (left; bottom sheet on mobile). Cards are collapsible.
- The Rp slider is logarithmic and spans the area around the feasible window.
- Drag the dot on the Rp–Cb plot to change Rp and extra pF. When focused: ↑/↓ = Rp, ←/→ = capacitance ±5 pF.
- Hover the waveform, or focus it and use ←/→, for a time/voltage tooltip.
- Invalid input (0, negative, empty, text, 1e12, …) never panics; an inline message appears under the field.

### Local build
```
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.22.2
cargo test
trunk build --release      # output: dist/ (built for the /i2c-pull/ subpath)
trunk serve                # dev server
bash tools/privacy_check.sh
```

### Privacy
No analytics, trackers, external fonts/CDNs, telemetry, server component or runtime network requests.
Your inputs never leave the browser and are not stored. The CSV file is generated locally.

### License
MIT — see `LICENSE`. Copyright i2c-pull contributors.
