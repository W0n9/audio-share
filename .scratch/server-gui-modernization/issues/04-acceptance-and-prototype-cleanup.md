# 04: 完成现代化验收与原型清理

**What to build:** 维护者与用户获得一套可复核的 Server GUI 现代化验收结果；正式工作树只保留选定实现，原型完整保存在独立分支作为设计主源。

**Blocked by:** 02 — 将主窗口实现为 Variant C 极简专注；03 — 将 Settings 实现为分组展开结构

**Status:** ready-for-human

- [ ] 人工验收清单以 Variant C 为当前设计，覆盖主窗口、Settings、异步状态、错误、banner、配置与现有生命周期行为。
- [ ] 在可执行平台检查并记录 Light、Dark、System 的主题表现、主题重启持久化和系统主题切换。
- [ ] 在可执行平台保留 `420×640` 与 `380×560` 的主窗口和 Settings 明暗截图；Windows 或 Linux 未执行项目明确标记，不伪造证据。
- [ ] en-US 与 zh-CN 无缺失文案，键盘焦点、展开区域、About 和减少动态效果经过人工检查。
- [ ] 现有前端检查、正式构建、Rust 配置/协议测试按项目流程执行，确认 Server Core、Wire Protocol 与 Android Client 未改变。
- [ ] 原型主源保存在 `prototype/server-gui-modernization` 分支，实施记录引用该分支。
- [ ] 正式工作树移除落选变体、方案切换器和开发原型入口，生产构建不包含原型代码。
- [ ] 所有实际执行结果与未完成的人工验收项均如实记录。

## Comments

- 2026-11-11: code-complete on `ticket/gui-mod-04-acceptance` (commit `53a0b25`), merged fast-forward into `integration/server-gui-modernization`. Acceptance record: `docs/server-rewrite/gui-modernization-acceptance.md` with 20 real WebKitGTK screenshots (Xvfb + xdotool headless run; `420×640`/`380×560`, en/zh, light/dark, error/banner/disclosure states). Agent-executed: theme restart persistence (UI toggled Dark → relaunch → still Dark), i18n parity (119/119 keys), prototype-branch integrity (`prototype/server-gui-modernization` @ `604f0d5` untouched), production tree free of variant switcher/prototype entries (`git ls-files`/`git grep` evidence). All npm/cargo checks green; Server Core, Wire Protocol and `android-app/` untouched (zero diff).
- Remaining human release gates (stay `ready-for-human` until recorded): Windows full suite; real-desktop tray flows; live system-theme switching (Xvfb has no settings daemon); `prefers-reduced-motion` observation; Starting/Stopping transitional visuals (server completes bind in ~18 ms, not capturable headlessly); Android v0.3.4 real-device regression.
- 2026-11-11 (Windows 11 background pass): maintainer ran the CI run 42971305804 portable exe (sha256 `4b8ec767…78af`, 12,030,464 bytes — matches the artifact); the ticket agent verified the embedded frontend and drove the app without taking the foreground (`PostMessage` clicks + `PrintWindow` captures). Recorded 16 screenshots under `docs/server-rewrite/img/gui-mod-04-win-*`: Light / Dark / System (OS in dark mode), `420×640` and `380×560` main + Settings in both themes, Starting / Stopping transitions (captured on the real desktop), start-failure alert, grouped Settings, About, Update group, and the Windows-only System rows (Sound Panel invoked and reported "声音面板已打开。"; Repair Firewall deliberately not clicked to avoid UAC). Theme edits wrote `theme=light|dark|system` to the exe-adjacent `config.ini`; the app was returned to `theme=system`, network config collapsed, and its original window position. Still `ready-for-human`: tray interaction, restart persistence (theme/language), live OS theme switch, reduced motion, full keyboard pass, UAC firewall repair, real update check (a background click produced no visible result), Windows 10, and the Android v0.3.4 device regression.
