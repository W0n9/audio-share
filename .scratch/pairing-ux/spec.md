# 配对、撤销与重新配对的交互

Status: code-complete (2026-11-23); pending code review
Feature: pairing-ux

## 问题

v0.6.0 的配对流在“重新配对”和“撤销后重新配对”两处让用户摸不着头脑。经代码与文案核对，
这不是单一控件的顺序问题，而是三层叠加：

1. **模型层**没有区分 Revocation / Delete / Forget / Re-pairing，代码只实现了 revoke；
2. **两端状态层**各说各话：手机仍显示 PAIRED，冷启动还会自动重连已撤销设备；
3. **控件与文案层**把服务端视角的措辞用在手机上，失败恢复的主按钮指向了无效动作。

### 当前行为（tag `v0.6.0`）

| 场景 | 手机端 | 服务端 GUI | 结果 |
|---|---|---|---|
| 撤销（在线） | SESSION 错误卡 204 | 设备行变 revoked，会话 ≤5s 关闭 | Home 仍显示 PAIRED；回 Home 后撤销提示消失 |
| 撤销后重配（QR） | 扫码，覆盖同一条凭据 | 需手动回顶部刷新二维码 | 同一行**静默**从 revoked 变回 active |
| 撤销后重配（SAS） | 每次新身份密钥 | 列表出现两条同名记录 | 无删除入口，记录永久堆积 |
| 配对失败 | 主按钮“Scan again” | 二维码区只剩“Refresh QR” | 服务端未刷新时，手机再扫必然再失败 |
| 冷启动（已撤销） | 自动连接 → 再次 204 | — | 每次启动重复一次失败 |

证据：`ControlSessionService.kt:243,273`、`pairing.rs:1086-1094`、`auth_store.rs:532-551`、
`AppState.kt:203-219,276-290,616-631`、`AudioShareApp.kt:244-268,972`、`PairingDialog.svelte:208-235`、
`docs/wire-protocol-v2/pairing.md:314-316`。

## 决定（2026-11-23 逐条确认）

1. **撤销可恢复，但只能显式 Restore。** 服务端在 approve 前发现同身份记录为 revoked 时，
   必须确认“恢复此设备？”，普通 Accept 不再静默复活。
2. **同一台手机重配保持同一身份。** QR 继续按服务端 pin 派生别名；SAS 在输入地址命中已存
   记录时复用该记录的 `keyAlias`。
3. **手机持久化“需要重新配对”状态。** 由要求重配的 surfaced close 置位，认证或配对成功后
   清除；Home 与 Settings 显示；`AutoConnectPolicy` 据此不再自动连接。
4. **“忘记此 Server”只删本地。** 凭据记录 + 身份密钥 + 匹配的地址字段，停止该会话；不静默
   删除，不需要服务端通道。
5. **配对成功删除同 `host:port` 的其他记录。** 用户已显式完成一次配对，旧地址记录不再保留；
   不同 Server pin 的记录不自动替换。
6. **服务端设备行：先撤销，后删除。** active 行只有撤销；revoked 行提供“重新配对”（打开新的
   配对邀请）与“删除”。`as-cmd` 增加 `clients delete <id>`。
7. **手机重配是专用流程。** 保留原因，固定提示“先让服务端操作员开始配对”，地址预填，
   QR/SAS 二选一；不再落入无上下文的通用配对页。
8. **失败恢复由服务端刷新主导。** 手机进入阻断等待态，扫码动作保留但文案先要求服务端刷新；
   服务端失败态把“刷新二维码”置前，并允许直接改为“使用配对码”。
9. **术语按 GLOSSARY 定稿**：Revocation / Restore / Delete (device record) / Forget Server /
   Re-pairing（见 `GLOSSARY.md`）。
10. **不改线协议。** 全部改动限于 Server Core、Server GUI 与 Android Client。
    见 `docs/adr/0006-revocation-and-repairing.md`。

## 目标流程

### 撤销后重新配对（QR）

1. 操作员在设备列表的 revoked 行点“重新配对”，打开新的配对邀请（QR + SAS 窗口）。
2. 手机在 Server 卡片上看到“需要重新配对”，点“重新配对”，看到“请在服务端开始配对后扫描”。
3. 手机扫码；服务端已连接卡片显示“正在恢复 <name>”。
4. 手机确认；服务端 approve 前确认“恢复此设备？”，确认后同一 `client_id` 回到 active。
5. 手机清除 needs-repairing，进入 SESSION。

### 撤销后重新配对（SAS）

同上，但第 3 步用配对码；手机复用已存记录的 `keyAlias`，因此服务端看到的是同一身份，
走同一套 Restore 确认，不会新增第二条记录。

### 忘记此 Server

Server 卡片或 Settings 提供“忘记此 Server”，二次确认后删除凭据与密钥、清空匹配地址、停止
会话，回到未配对 Home。

### 配对失败

手机失败卡改为阻断态：说明邀请已失效，要求服务端刷新后再扫描；服务端失败态把“刷新二维码”
置前，并显示“使用配对码”。

## 行为变化清单

- Server Core：approve 时识别 revoked 同身份记录并返回待确认状态；新增 delete。
- Server GUI：revoked 行 + Restore/Delete；行内“重新配对”；失败态按钮顺序；配对成功反馈。
- `as-cmd`：`clients delete <id>`（与 GUI 同一 Core）。
- Android Client：`CredentialRecord` 增加 needs-repairing；`CredentialStore.delete` 接入；
  `AutoConnectPolicy` 读取该状态；专用重配入口；错误卡与 Home/Settings 文案；配对成功后清理
  同地址记录。
- 文档：`GLOSSARY.md`、ADR-0006、本 spec。

## 票据

按依赖切成五个 tracer-bullet 票据：

| 票据 | 内容 | Blocked by |
|---|---|---|
| [01](issues/01-server-core-restore-delete.md) | Server Core：Restore 确认 + Delete + `as-cmd clients delete` | — |
| [02](issues/02-server-gui.md) | Server GUI：设备行、重配入口、失败态、成功反馈 | 01 |
| [03](issues/03-android-repair-state.md) | Android：needs-repairing、Forget、自动连接闸门、文案 | — |
| [04](issues/04-android-repair-flow.md) | Android：专用重配入口、SAS 密钥复用、失败阻断态 | 01, 03 |
| [05](issues/05-closeout.md) | 文档、测试与证据收口 | 02, 04 |

合并 SHA、命令结果与证据边界见 `evidence/README.md`。

## 文案定稿

**Android**（当前只有 `values/` 英文）

| Key | 文案 |
|---|---|
| Home 已配对次要动作 | `Re-pair`（替代 `Pair a new phone` 与独立的 `Scan QR instead`） |
| Server 卡片状态 | `Needs re-pairing` |
| Settings 空态 | `Pair a Server`（替代 `Pair a phone`） |
| 204 错误卡按钮 | `Re-pair` |
| 重配页提示 | `Ask the Server operator to start pairing. Then scan the new QR code or use the pairing code.` |
| Forget 动作 | `Forget this Server` |
| Forget 确认 | `Forget {name}? The saved pairing and its key are deleted from this phone.` |
| 失败阻断态 | `The invitation can no longer be used. Ask the Server operator to refresh the QR code, then scan the new one.` / 按钮 `Scan new code` |

**Server GUI**（en / zh）

| Key | en | zh |
|---|---|---|
| revoked 行 | `Re-pair` / `Delete` | `重新配对` / `删除` |
| 删除确认 | `Delete {name}'s record? It is removed permanently; this cannot be undone.` | `删除 {name} 的记录？该记录将被永久移除，无法撤销。` |
| 恢复确认标题 | `This device was revoked` | `此设备已被撤销` |
| 恢复确认正文 | `Restoring gives the saved pairing access again. No new key is created.` | `恢复后该配对将重新生效，不会生成新密钥。` |
| 恢复确认按钮 | `Restore device` | `恢复设备` |
| 配对成功 | `{name} is paired.` | `{name} 已配对。` |

## 验收

- Rust host 测试：revoked 同身份 pairing → 待确认；确认后原 `client_id` active；拒绝则保持
  revoked 且无新记录；delete 只允许 revoked。
- JVM 测试：needs-repairing 置位/清除；需要重配时 `AutoConnectPolicy` 不自动连接；Forget 删除
  凭据与密钥；配对成功清理同地址记录；SAS 复用地址命中的 `keyAlias`。
- GUI：`svelte-check` + 生产构建通过；revoked 行的两个动作与确认文案由代码审阅覆盖
  （`gui/` 没有 Svelte/store 测试运行器）。
- 设备（2026-11-23 实况）：模拟器已跑 Keystore、SAS 配对、认证、204 撤销、SAS 重配
  Restore（同一 `client_id`、同一 `created_at`、单条 active 记录），标 `EMULATOR`；
  “首次配对 → 撤销 → QR 重配恢复 → 撤销 → SAS 重配恢复 → Forget”的完整 UI 清单因没有
  Re-pair/Restore/Forget 的自动化 UI 路径而标 `NOT RUN`。真机 `NOT RUN`（未排到会话）。
  既有 `pairSas` 设备用例在 `COMPLETE` 断言上失败，原因是配对交接会清空快照；该清理路径在
  `v0.6.0..d28e099` 之间未变（未在 `v0.6.0` 构建上复跑）。配对与 Restore 已由 `reload`
  与 `clients list` 独立确认。`pairQr`、QR 重配、Forget 与真机均为 `NOT RUN`。
  详见 `evidence/README.md`。

## 不在本轮

- 修改已签核的 `pairing.md` / `spec.md`（只有身份策略变更才需要）。
- 多 Server 选择器、账号、服务端主动通知客户端、跨网络发现。
- stable 阶段的设备管理重做（拆分“配对/设备”两个入口）。
