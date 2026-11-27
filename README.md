# I2C Pull

브레드보드 I2C 진단기 — 내 브레드보드 회로를 화면에 다시 꽂아 보고, I2C 통신이 **되는지/불안정한지/안 되는지**와 그 이유를 한눈에 봅니다.
Rust → WebAssembly, 100% 브라우저 내 실행.
Breadboard I2C checker — rebuild your breadboard on screen and see at a glance whether I2C **works, is unstable, or fails**, and why.
Rust → WebAssembly, runs 100% in the browser.

[한국어](#한국어) · [English](#english)

---

## 한국어

### 쓰는 법 (브레드보드 흐름)
1. **부품 올리기** — 오른쪽 팔레트(모바일은 하단 시트)에서 부품을 누르거나 보드로 끌어다 놓으면
   VCC·GND·SDA·SCL 줄에 자동으로 꽂힙니다. 선을 직접 그릴 필요가 없습니다.
   - MCU 보드: 아두이노 우노(5V), ESP32(3.3V), 라즈베리파이(3.3V, 1.8kΩ 풀업 내장)
   - 센서 모듈: BME280, MPU6050, SSD1306 OLED, VL53L0X, "10k 풀업 달린 모듈", 맨 칩(3.3V 전용)
   - 풀업 저항(E12 값 선택), 점퍼선(5/10/20/30/50 cm 선택), 레벨 시프터
   - 각 모듈은 흔한 브레이크아웃 기준의 온보드 풀업·핀 용량·I2C 주소를 갖고 있으며 **자세히** 패널에서 수정할 수 있습니다.
   - 보드 위 부품을 누르면 선택되고(이름·풀업·주소 표시) **빼기** 버튼이나 Delete 키로 뺍니다(키보드: 보드 포커스 후 ←/→로 선택).
   - SDA/SCL 점퍼선은 고른 길이만큼 커지는 고리로 그려집니다.
   - **3D 보드** (WebGL2): 책상 위 브레드보드, MCU 보드(우노·ESP32·라즈베리파이), 센서 모듈, 색띠가 값과 맞는 풀업 저항, 점퍼선이
     코드로 만든 로우폴리 모델로 보입니다. 통신이 되면 SDA/SCL 선을 따라 빛 펄스가 흐르고, 불안정이면 펄스가 끊기며 일부가 주황색이 되고,
     안 됨이면 문제 부품에서 멈추고 그 부품이 빨갛게 깜빡입니다(겹친 온보드 풀업 SMD 저항도 빨갛게). OLED 화면에는 "I2C OK" 또는 "--".
     움직임 줄이기 설정이면 정지된 빛만 보입니다.
   - **조작**: 빈 곳 왼쪽 끌기 = 회전, 오른쪽 끌기(또는 Shift+끌기)·두 손가락 = 이동, 휠·핀치 = 확대/축소(관성 있음),
     빈 곳 두 번 누르기 = 처음 시점. 오른쪽 위 **원근 / 위 / 옆** 버튼으로 시점 전환. 마우스를 올린 부품은 밝아지고,
     팔레트에서 끌어오는 동안 다음 빈 칸에 반투명 고스트가 붙습니다.
   - WebGL2를 못 쓰는 브라우저에서는 기존 2D 보드와 짧은 안내가 나옵니다.
2. **판정 보기** — 화면 맨 위에 큰 글씨로 **통신 됨 / 불안정 / 안 됨**(초록/주황/빨강), 쉬운 말로 된 이유 하나와 고칠 방법 하나가 나옵니다.
   예: "모듈 풀업이 겹쳐 합성 915Ω — 센서가 LOW를 못 만들어요. → MPU6050 모듈의 풀업 저항을 떼세요."
3. **신호 보기** — 보드 아래 오실로스코프 띠가 선택한 속도의 SCL 파형을 아주 느리게 흘려 보여줍니다.
   상승 모서리는 실제 RC 곡선(`wave_v`)이며, VIH에 닿기 전에 떨어지는 비트는 빨갛게 깜빡입니다.
4. **비교** — "이 회로 저장"으로 지금 보드를 A로 고정하고, 부품이나 값을 바꾸면 A와 지금의 판정·풀업 합성·상승 시간·버스 용량이 나란히 보입니다(메모리에만 저장, 새로고침하면 사라짐).
5. **자세히** (기본 접힘) — 기존 계산기: 버스 값(pF/cm, 브레드보드 기생 용량, VOL, IOL) 수정, 부품별 풀업/핀 용량/주소/5V 허용 수정,
   Rp_min~Rp_max, 추천 E12 값, 상승 시간 파형, Rp–Cb 허용 영역 그래프, CSV 내보내기.

### 프리셋 (예시 값, 수정 가능)
| 프리셋 | 판정 | 이유 |
|---|---|---|
| 아두이노 우노 + BME280 하나 (됨) | 통신 됨 | 10kΩ 하나, 100 kHz에서 여유 충분 |
| ESP32 + 모듈 4개 풀업 겹침 (안 됨) | 안 됨 | 2.2k ∥ 4.7k ∥ 4.7k ∥ 4.7k = 915Ω < Rp_min 967Ω |
| 라즈베리파이 + 50cm 선 + OLED 400kHz (불안정) | 불안정 | 50 cm + 헤더에서 브레드보드까지 10 cm = 선 합계 60 cm > 50 cm @ 400 kHz |
| 5V 풀업 + 3.3V 센서 (손상 위험) | 안 됨 | 5V 풀업이 3.3V 전용 칩에 걸림, 레벨 시프터 없음 |

### 판정 규칙 (`ip_core::verdict`, 단위 테스트 있음)
위에서부터 우선순위 순서이며, 첫 번째로 걸린 항목이 이유/고칠 방법으로 표시됩니다.
- MCU 없음, 통신할 센서 없음 → 안 됨
- 5V 풀업 + 3.3V 전용 부품 + 레벨 시프터 없음 → 안 됨 (손상 경고)
- 같은 고정 주소 중복(MPU6050 두 개 0x68, VL53L0X 두 개 0x29 …) → 안 됨, 충돌 주소 표시
- 풀업 없음 → 안 됨
- 합성 풀업 < Rp_min (싱크 전류 규격 초과) → 안 됨
- 총 버스 용량 > 400 pF → 안 됨
- 상승 시간 > tr_max → 안 됨, tr_max의 80–100 % → 불안정
- 400 kHz 이상에서 점퍼선 합계 > 50 cm → 불안정

### 모델 / 수식 (I2C-bus specification의 표준 추정식)
| 항목 | 식 |
|---|---|
| 최소 풀업 | Rp_min = (VDD − VOL) / IOL |
| 상승 시간 (0.3·VDD → 0.7·VDD) | tr = 0.8473 · Rp · Cb |
| 최대 풀업 | Rp_max = tr_max / (0.8473 · Cb), tr_max = 1000 ns (Sm), 300 ns (Fm), 120 ns (Fm+) |
| 최대 클럭 추정 | f_max ≈ 1 / (tLOW_min + tHIGH_min + tr + tf), tf = 20 ns 고정 가정 |
| 정적 전력 (라인당, LOW 유지) | P = VDD² / Rp |
| 합성 풀업 | 1/R_eff = Σ 1/R_i (MCU·모듈·저항·레벨 시프터의 풀업 전부) |
| 버스 용량 | Cb = Σ 핀 용량 + 선 길이 합계 × pF/cm + 브레드보드·커넥터 용량 |

### 한계 (정직한 안내)
계획용 추정치일 뿐입니다. 모든 풀업은 MCU 로직 전압에 연결된 것으로, 모든 부품은 한 버스 구간에 있는 것으로 봅니다.
레벨 시프터는 5V 위험만 없애고 자기 풀업·핀 용량을 더하는 것으로 단순화했습니다.
직렬 저항, 누설 전류, 개별 부품의 입력 임계값, 링잉, 크로스토크는 고려하지 않습니다. 부품 값은 흔한 모듈 기준 예시이니 내 보드를 확인하고,
실제 상승 시간은 반드시 오실로스코프로 측정하세요.

### 로컬 빌드
```
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.22.2
cargo test
cargo clippy --workspace --all-targets -- -D warnings
trunk build --release      # 결과물: dist/ (/i2c-pull/ 하위 경로용)
trunk serve                # 개발 서버
bash tools/privacy_check.sh
```

### 프라이버시
분석 도구, 트래커, 외부 폰트/CDN, 텔레메트리, 서버 구성요소, 런타임 네트워크 요청이 없습니다.
회로와 입력값은 브라우저 밖으로 나가지 않으며 저장되지도 않습니다. CSV는 브라우저 안에서 생성됩니다.

### 라이선스
MIT — `LICENSE` 참고. Copyright i2c-pull contributors.

---

## English

### How to use (breadboard flow)
1. **Place parts** — tap a part in the palette (right; bottom sheet on mobile) or drag it onto the board. It snaps onto the
   VCC/GND/SDA/SCL lines automatically; no free wiring.
   - MCU boards: Arduino Uno (5 V), ESP32 (3.3 V), Raspberry Pi (3.3 V, 1.8 kΩ on-board pull-ups)
   - Sensor modules: BME280, MPU6050, SSD1306 OLED, VL53L0X, "module with 10k pull-ups", bare chip (3.3 V only)
   - Pull-up resistor (pick an E12 value), jumper wire (pick 5/10/20/30/50 cm), level shifter
   - Every module carries typical on-board pull-ups, pin capacitance and I2C address as presets, editable in **Details**.
   - Tap a part on the board to select it (name, pull-up, address), then **Remove** or press Delete (keyboard: focus the board,
     ←/→ to select).
   - SDA/SCL jumpers are drawn as loops that grow with the chosen wire length.
   - **3D board** (WebGL2): a breadboard on a desk with low-poly models generated in code — MCU boards (Uno, ESP32, Raspberry Pi),
     sensor modules, pull-up resistors whose colour bands match the value, jumper wires. When the bus works, glowing pulses travel
     along SDA/SCL; when it is unstable they stutter and some turn amber; when it fails they stop at the culprit part, which pulses red
     (stacked on-board SMD pull-ups turn red too). The OLED screen shows "I2C OK" or "--". With reduced motion the glow is static.
   - **Controls**: left-drag on empty space = orbit, right-drag (or Shift+drag) / two fingers = pan, wheel / pinch = zoom (with
     inertia), double-click / double-tap on empty space = reset. **3D / Top / Side** buttons (top right) switch views. Hovered parts
     light up, and while you drag from the palette a translucent ghost snaps to the next free slot.
   - Browsers without WebGL2 get the previous 2D board with a short note.
2. **Read the verdict** — at the top, in big letters: **Works / Unstable / Fails** (green/amber/red) with one plain-language reason and one fix.
3. **Watch the signal** — a scope strip under the board plays the SCL square wave at the chosen speed, slowed down a lot. The rising
   edge is the real RC curve from `wave_v`; bits that fall before reaching VIH flash red.
4. **Compare** — "Save this board" pins the current board as A; change any part or value to see A vs now side by side (verdict,
   combined pull-up, rise time, bus capacitance). Kept in memory only.
5. **Details** (collapsed by default) — the original calculator: bus values (pF/cm, breadboard stray, VOL, IOL), per-part pull-up /
   pin capacitance / address / 5 V-tolerance, Rp_min–Rp_max, E12 pick, rise-time waveform, Rp–Cb window plot, CSV export.

### Presets (illustrative values, editable)
| Preset | Verdict | Why |
|---|---|---|
| Arduino Uno + one BME280 | Works | single 10 kΩ, plenty of margin at 100 kHz |
| ESP32 + 4 modules, stacked pull-ups | Fails | 2.2k ∥ 4.7k ∥ 4.7k ∥ 4.7k = 915 Ω < Rp_min 967 Ω |
| Raspberry Pi + 50 cm wire + OLED at 400 kHz | Unstable | 50 cm + 10 cm header-to-breadboard jumper = 60 cm total > 50 cm at 400 kHz |
| 5 V pull-up + 3.3 V sensor | Fails (damage risk) | 5 V pull-ups on a 3.3 V-only chip, no level shifter |

### Verdict rules (`ip_core::verdict`, unit-tested)
In priority order; the first hit is the reason/fix shown.
- no MCU, nothing to talk to → fails
- 5 V pull-up with a 3.3 V-only part and no level shifter → fails (damage warning)
- duplicate fixed address (two MPU6050 at 0x68, two VL53L0X at 0x29, …) → fails, naming the clash
- no pull-up at all → fails
- combined pull-up below Rp_min (sink current over spec) → fails
- total bus capacitance over 400 pF → fails
- rise time over tr_max → fails; 80–100 % of tr_max → unstable
- jumpers over 50 cm in total at 400 kHz or faster → unstable

### Model / formulas (standard I2C-bus specification estimates)
| Quantity | Formula |
|---|---|
| Minimum pull-up | Rp_min = (VDD − VOL) / IOL |
| Rise time (0.3·VDD → 0.7·VDD) | tr = 0.8473 · Rp · Cb |
| Maximum pull-up | Rp_max = tr_max / (0.8473 · Cb), tr_max = 1000 ns (Sm), 300 ns (Fm), 120 ns (Fm+) |
| Max clock estimate | f_max ≈ 1 / (tLOW_min + tHIGH_min + tr + tf), tf fixed at 20 ns |
| Static power (per line, held low) | P = VDD² / Rp |
| Combined pull-up | 1/R_eff = Σ 1/R_i (every pull-up on MCU, modules, resistors, shifter) |
| Bus capacitance | Cb = Σ pin capacitance + total wire length × pF/cm + breadboard/connector stray |

### Limitations (honesty note)
Planning estimate only. All pull-ups are assumed to go to the MCU logic rail and all parts share one bus segment. A level shifter
only clears the 5 V hazard and adds its own pull-ups and pin capacitance. Ignores series resistors, leakage, input thresholds of
specific parts, ringing and crosstalk. Part values are typical module examples — check your own boards and measure the real rise
time with a scope.

### Local build
```
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.22.2
cargo test
cargo clippy --workspace --all-targets -- -D warnings
trunk build --release      # output: dist/ (built for the /i2c-pull/ subpath)
trunk serve                # dev server
bash tools/privacy_check.sh
```

### Privacy
No analytics, trackers, external fonts/CDNs, telemetry, server component or runtime network requests.
Boards and inputs never leave the browser and are not stored. The CSV file is generated locally.

### License
MIT — see `LICENSE`. Copyright i2c-pull contributors.
