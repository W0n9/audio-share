# 20: 会话屏 Direction A

**What to build:** SESSION 屏按方向 A 呈现四种 outcome：Live 四阶段、Reconnecting、Ended(User stop)、Ended(Surfaced)；错误与重连原位显示，主操作唯一。

**Blocked by:** 15, 18。

**Status:** resolved

- [x] Live 的 CONNECTING/AUTHENTICATING/READY/PLAYING 各有清晰状态与文案。
- [x] Reconnecting 显示 attempt 与 lastCloseCode，停留原屏，Stop 可用。
- [x] Ended(UserStop) 显示“you stopped” + Connect again；Ended(Surfaced) 显示错误卡 + Retry/Pair again，二者视觉可区分。
- [x] Stop 是唯一主操作；不出现暂停/恢复。
- [x] 错误卡与状态行带 liveRegion；触控目标、200% 字体、zh-CN 文案齐备。
- [x] Robolectric 测试覆盖四种 outcome 与两种 Ended 的差异；08 场景 #1–#5 可断言。

## Comments

2026-11-17 — 完成并合入集成分支（merge `9ed933f`，feature `d3f2609`）。
四种 outcome、Reconnecting attempt + lastCloseCode、两种 Ended 的差异化呈现、Stop 唯一主操作、
liveRegion/48dp/资源化（en+zh 各 118 strings + 1 plural）。JVM 223 tests/variant PASS。
证据：`evidence/ticket-20/`。

裁定：本票要求 Reconnecting 显示 attempt 与 lastCloseCode，优先于工单 01 中
「lastCloseCode 仅诊断、不展示」的措辞（实现票在后且更具体）；以诊断行呈现。
200% 字体/RTL/TalkBack 的视觉与播报检查为 `EMULATOR`/手工项，记 NOT RUN，留给 26/27。
