# 配对码失效恢复与确认模型：一手资料市场调研

调研日期：2026-11-17  
范围：消费电子/即时通讯/网络接入/协议规范中的配对码、二维码、SAS 与设备批准流程。  
方法：优先抓取厂商官方帮助页、开发者文档、协议规范与源码；无法直接抓取时，使用搜索引擎返回的同一官方页面摘要，并在文中标注。  
标记约定：**【明确】** = 来源正文直接写明；**【推断】** = 根据来源正文可合理推出但未逐字写明；**【NOT FOUND】** = 未找到一手来源，不用二手材料补细节。  
说明：WhatsApp、Signal、Telegram 的官方页面在本次环境中直接抓取返回空内容，以下依据搜索引擎索引到的官方页面摘要，并明确标注；未把第三方教程当作事实依据。

## 1. 结论摘要

1. **主流恢复方式不是“继续展示旧码”，而是“旧码立即失效 + 明确下一步”。** 常见有三类：定时自动换码（Telegram 文档明确每约 30 秒重新生成并自动刷新 QR）、过期/失败后显式刷新或重开（RFC 8628、Windows Phone Link、Matter Open Commissioning Window、Philips Hue 重新按按钮）、以及长期静态码 + 重试（HomeKit 的 8 位 setup code 印在设备/包装上）。没有找到任何一手来源显示：配对码过期后界面继续展示旧码并鼓励用户再扫一次。
2. **“静默自动刷新”不是通行默认。** RFC 8628 §3.5 明确：`expired_token` 后客户端 MAY 发起新的 device authorization request，但 SHOULD wait for user interaction before restarting；§3.1 还说客户端 SHOULD only commence when prompted by the user and not automatically。明确写“自动重新生成”的是 Telegram QR 登录。WhatsApp Web 的“约 30–60 秒自动刷新”只有二手教程，官方帮助页未给时限，本文标为 NOT FOUND。
3. **一次性语义上，本项目比大多数产品更严格。** 本项目 nonce 被“第一个格式正确的 PairingRequest”消耗，不论成败；OAuth device flow 是 TTL + 轮询，Matter 是限时 commissioning window（失败可在窗口内重试，窗口关闭后重开），HomeKit 是长期静态码。明确写短时过期且一次性使用的是 Telegram 的 login token；Tailscale 的 one-off auth key 也明确“使用后 revoke”；Windows Phone Link 的 QR 明确 3 分钟过期（是否单次未写）。本项目的特殊之处是“首个请求即消耗，不论成败”，而不是单纯的 TTL。
4. **确认模型的主流是“单端完成 + 密码学证明”。** HomeKit（手机输入 8 位码，SRP）、Matter（commissioner 扫设备码，PASE/SPAKE2+）、AirPlay（接收端显示密码，发送端输入）、Google Cast（电视显示 QR，手机扫）、WhatsApp/Signal/Telegram（手机端授权）、Tailscale（auth key 或管理员批准）都不要求两端同时做一次人工确认。
5. **双端显式确认集中在“没有带外公钥绑定、必须靠人比对”的场景。** Bluetooth Secure Simple Pairing 的 Numeric Comparison 要求两端显示同一个 6 位数字并各自确认；KDE Connect 要求另一端接受请求并比对验证码；Windows Phone Link 有 code matching 且失败三次会阻止继续；Syncthing 要求双方都加入对方 Device ID。它们共同点是：人比对本身就是安全机制，不是重复劳动。
6. **服务端“批准设备”列表是另一类先例。** Tailscale 的设备批准由 Owner/Admin 在 admin console 完成，设备端只显示“等待批准”；这对应本项目服务端的 “Accept device”，而不是 SAS 比对。对本项目而言：QR 模式已经用二维码里的服务端公钥做了 pinning，SAS 的密码学必要性弱于 SAS fallback 模式；保留服务端 Accept 作为“授权/防误扫”更有一手先例支持，手机端 Confirm 是否保留则取决于对 `pairing.md` §7.3 适用范围的裁决。

## 2. 逐产品/规范

### 2.1 Apple HomeKit / Home app

**配对流程**

- 【明确】Home app 添加配件：打开 Home app → Add → Add Accessory → “scan a code or hold your device near the accessory”；随后分配房间、命名，点 Continue → Done。扫描不到时可走 More options。([Apple Support: Add a smart home accessory to the Home app](https://support.apple.com/en-us/104998)，访问 2026-11-17)
- 【明确】配件上可能是 **8 位 Apple Home setup code**；Matter 配件可能是 **11 位或 21 位数字码**，也可能是 QR code；Apple Home 支持的智能电视会显示 QR code。([Apple Support: Set up accessories with Home on iPhone](https://support.apple.com/en-gu/guide/iphone/iph125110541/ios)，访问 2026-11-17)
- 【明确】HomeKit 配对用 **3072-bit SRP** 交换密钥，setup code 是“an eight-digit code provided by the accessory's manufacturer, entered on the user's device”；Matter 配件用 **SPAKE2+** 和厂商提供的 PIN，并做 Device Attestation。([Apple Platform Security: HomeKit communication security](https://support.apple.com/guide/security/communication-security-sec3a881ccb1/web)，访问 2026-11-17)

**有效期与单次性**

- 【明确】setup code 印在配件本身、包装或文档上，不是会话级倒计时码。
- 【NOT FOUND】Apple 官方帮助页没有写 setup code 的过期时间，也没有写“配对失败后该码立即失效”。【推断】它是长期静态凭据，安全性来自“必须物理接触设备/包装”以及 SRP，而不是短时一次性。
- 【推断】失败后没有“刷新码”动作；Apple 的故障页建议重置配件、检查 2.4 GHz、把配件靠近路由器、检查 home hub。([Apple Support: If you can't add a smart home accessory to the Home app](https://support.apple.com/en-us/126198)，访问 2026-11-17)

**确认模型**

- 【明确】用户只在 iPhone/iPad 上操作 Home app；文档没有配件端“确认配对”的步骤。
- 【推断】单端确认：手机输入/扫描 setup code 即完成授权，没有 SAS、没有两端比对、没有服务端操作员批准。
- 【推断】Home app 的 “Continue / Done” 是命名与落库，不是第二端的安全确认。

**对恢复/等待状态的启示**

- 静态码产品把“失败恢复”交给重试和重置，不提供“刷新码”；用户不会被旧码困住，因为码本身不会过期。
- 这与本项目“短时 nonce + 一次性”不同，不能直接照搬；可借鉴的是“失败后给出明确的重置/重试路径”，而不是让用户面对一个已经无效的码。

### 2.2 Apple AirPlay（Apple TV / Mac）

**配对流程**

- 【明确】第一次用 AirPlay 连接 Apple TV 时，电视上出现 **onscreen password**，用户在发送端设备输入。([Apple Support: If you're asked to enter an AirPlay password on Apple TV 4K and Apple TV HD](https://support.apple.com/en-us/102324)，访问 2026-11-17)
- 【明确】从 Mac 投屏到电视时，如被要求，在 Mac 上输入 **four-digit code**。([Apple Support: Stream video and audio with AirPlay](https://support.apple.com/en-tm/guide/mac-help/mchld7e543a0/mac)，访问 2026-11-17)
- 【明确】Apple TV 的 AirPlay 安全选项包括 Everyone、Anyone on the Same Network、Only People Sharing This Home、Require Password；第三代 Apple TV 还有 **Require Device Verification**，要求所有 AirPlay 设备使用一次性 onscreen password。([Apple Support 102324](https://support.apple.com/en-us/102324)，访问 2026-11-17)

**有效期与单次性**

- 【明确】4K/HD 页面只说“第一次”会出现 onscreen password；没有给出倒计时或过期时间。
- 【明确】第三代 Apple TV 的 Require Device Verification 被描述为 “one-time onscreen password for all AirPlay-enabled devices”。
- 【NOT FOUND】4K/HD 的 onscreen password 是否严格一次性、多久过期，官方页面未写。

**确认模型**

- 【明确】码显示在接收端（电视），输入在发送端。
- 【推断】单端确认：输入正确即开始串流；电视端没有“接受/拒绝”二次点击。安全策略（同网络、同家庭、Require Password）在连接前生效。

**对恢复/等待状态的启示**

- 接收端显示、发送端输入的模型，和本项目“服务端显示 QR、手机扫描”同构；AirPlay 没有要求接收端操作员再点一次接受。
- 但 AirPlay 的码是连接凭据，不是设备身份 pinning；本项目 QR 模式已经用公钥 pinning，安全语义更强。

### 2.3 Google Cast / Chromecast

**配对流程**

- 【明确】Google Home app → Add → Device → 授予相机权限 → 扫描电视屏幕上的 QR code；也可以 “enter the setup code manually in the Home app instead of scanning the QR code”。([Google Support: Set up Chromecast with Google TV and Voice Remote](https://support.google.com/chromecast/answer/10109990?hl=en)，访问 2026-11-17)
- 【明确】完成后手机端显示 “Setup here is finished”，电视端显示 “Welcome. Your Chromecast with Google TV is ready!”。([同上](https://support.google.com/chromecast/answer/10109990?hl=en)，访问 2026-11-17)

**有效期与单次性**

- 【NOT FOUND】官方 setup 页面没有写 QR/setup code 的有效期，也没有写失败后码是否立即失效。
- 【推断】setup code 是设备进入 setup 模式时生成的会话码；具体生命周期未在官方消费者文档中说明。

**失败恢复**

- 【明确】官方故障页给出：重启 Chromecast、重启路由器、恢复出厂设置、联系支持；遥控器配对失败时关闭附近其他遥控器；手机找不到设备时连到同一 Wi-Fi，或手动连 Chromecast 广播的临时网络。([Google Support: Set up Chromecast with Google TV](https://support.google.com/chromecast/answer/10109990?hl=en)，访问 2026-11-17)
- 【明确】没有“点击刷新旧 QR”的步骤；文档导向重启/重新 setup。

**Guest mode（补充）**

- 【明确】Chromecast 电视的 guest mode 已不再支持：“Guest mode is no longer supported. Guests can cast content from their Android or iOS devices by connecting to the host's Wi-Fi network.” 旧流程只需要 Chromecast 提供的 Guest mode PIN。([Google Support: Set up and manage guest mode](https://support.google.com/chromecast/answer/6109286?hl=en)，访问 2026-11-17)
- 【明确】Chromecast Audio 的 guest mode 仍存文档：4 位 PIN 可通过不可听音频传输，失败时手动输入；PIN 在 Google Home app 的设备设置里查看。([Google Support: Set up guest mode for Chromecast Audio](https://support.google.com/chromecast/answer/6279388?hl=en)，访问 2026-11-17)

**确认模型**

- 【明确】电视只负责显示 QR/setup code；手机负责扫描/输入并完成 setup。
- 【推断】单端确认：电视端没有“Accept device”步骤。设备身份通过 setup 会话和 Google 账号绑定完成。

### 2.4 Matter / Thread commissioning

**配对流程**

- 【明确】Matter 配件使用 onboarding payload：QR code 或 **11/21 位 manual pairing code**；payload 含 passcode 与 discriminator。Apple 支持页明确列出 11 位或 21 位 Matter code。([Apple Support: Set up accessories with Home on iPhone](https://support.apple.com/en-gu/guide/iphone/iph125110541/ios)，访问 2026-11-17)
- 【明确】Commissioner 用 payload 里的 passcode 做 **PASE（Passcode Authenticated Session Establishment）**； commissioner 先通过 discriminator 匹配设备广告，再建立安全会话；随后做 attestation、NOC 安装、网络配置，最后发 `CommissioningComplete`。([Build with Matter: Commissioning](https://handbook.buildwithmatter.com/how-it-works/commisioning)，访问 2026-11-17；[Google Home Developers: Commissioning](https://developers.home.google.com/matter/primer/commissioning)，访问 2026-11-17)
- 【明确】commissioner 在 PASE 阶段 arm fail-safe；如果 commissioning 没有成功，fail-safe 会让设备回滚到原始状态。([Build with Matter: Commissioning](https://handbook.buildwithmatter.com/how-it-works/commisioning)，访问 2026-11-17)

**有效期与单次性**

- 【明确】Matter 使用“commissioning window”而不是永久可配。Matter 1.0 Core Spec 的 CM8 要求：commissioning 与 unsecured rendezvous 在未完成时 “time-out after at most 15 minutes”；Open Commissioning Window（OCW）的 `CommissioningTimeout` 参数以秒计，可短于 15 分钟上限。([Matter 1.0 Core Specification PDF](https://csa-iot.org/wp-content/uploads/2022/11/22-27349-001_Matter-1.0-Core-Specification.pdf)，访问尝试 2026-11-17；该 PDF 直接抓取返回 403，以上为搜索引擎索引到的规范正文)
- 【明确】chip-tool 的 `pairing open-commissioning-window <node_id> <option> <window_timeout> ...` 中，`<window_timeout>` 是 “time in seconds, before the commissioning window closes”。([Matter docs: Working with the CHIP Tool](https://project-chip.github.io/connectedhomeip-doc/development_controllers/chip-tool/chip_tool_guide.html)，访问 2026-11-17)
- 【推断】失败后可在窗口内重试；窗口关闭后需重新打开 commissioning window。规范没有把“第一次请求即消耗 passcode”写成本项目 nonce 那样的规则。
- 【明确】OCW 由现有 administrator 在 CASE 会话里触发：administrator 选择新的随机临时 passcode，只把 PAKE verifier 发给设备；新 passcode 通过带外方式呈现。([Matter 1.0 Core Spec PDF](https://csa-iot.org/wp-content/uploads/2022/11/22-27349-001_Matter-1.0-Core-Specification.pdf)，访问尝试 2026-11-17；搜索索引正文)

**失败恢复**

- 【明确】fail-safe 回滚；chip-tool 文档提供 `open-commissioning-window` 重新开窗。([Build with Matter](https://handbook.buildwithmatter.com/how-it-works/commisioning)；[CHIP Tool](https://project-chip.github.io/connectedhomeip-doc/development_controllers/chip-tool/chip_tool_guide.html)，访问 2026-11-17)
- 【明确】Alexa 的 Matter commissioning 最佳实践提到窗口 “closes after several minutes”，用户经常错过截止时间；这解释了为什么需要重新开窗，而不是继续用旧码。([Amazon Alexa+ Developer Docs: Best Practices to Commission Matter Devices](https://developer.amazon.com/docs/alexaplus/smarthome/best-practices-commission-matter)，访问 2026-11-17)

**确认模型**

- 【明确】commissioner（手机/音箱/App）扫描或输入设备码，设备只负责广告与 PASE；设备端没有“显示同一个码并让用户确认”的 SAS 步骤。
- 【推断】单端确认：commissioner 端完成设备选择、passcode 证明、attestation 与 NOC 安装；没有服务端操作员批准，也没有两端数字比对。

### 2.5 WhatsApp Web / Linked Devices

> 抓取说明：`faq.whatsapp.com` 的页面在本环境直接抓取返回空内容；以下依据搜索引擎返回的 WhatsApp Help Center 官方页面摘要，未使用第三方教程补细节。

**QR 流程**

- 【明确】新设备打开 WhatsApp → Linked devices → Link a device，显示 QR；主手机进入 Settings → Linked devices → Link a device，用生物识别或手机 PIN 验证后扫描 QR；如果设备无法识别，WhatsApp 会要求确认后再 linking。([WhatsApp Help Center: How to link a device](https://faq.whatsapp.com/1317564962315842)，访问 2026-11-17，经搜索索引)
- 【明确】主手机端是授权端；新设备只显示 QR 并等待。

**8 位配对码流程**

- 【明确】主手机：Linked devices → Link a device → **Link with phone number instead**；新设备显示 **8-character code**；主手机点 “Enter code to link new device” 通知，看到设备名称与位置，点 Confirm，生物识别/PIN 验证后输入 8 位码；若无法识别设备，会再问 “Don't link / Link device”。([WhatsApp Help Center: How to link a device with phone number](https://faq.whatsapp.com/1324084875126592)，访问 2026-11-17，经搜索索引)

**有效期与单次性**

- 【NOT FOUND】官方帮助页没有写 QR 的有效期、刷新间隔，也没有写 8 位码的有效期。
- 【NOT FOUND】官方帮助页没有写“配对失败后旧 QR 是否立即从界面消失”。
- 二手教程常称 WhatsApp Web QR 约 30–60 秒过期并自动刷新；这些不是官方来源，本文不把它当作事实。

**失败恢复**

- 【NOT FOUND】官方页面没有专门的“配对失败后如何刷新 QR”步骤；可确认的是：linking 需要主手机端生物识别/PIN，且未识别设备会被要求再次确认。
- 【推断】主手机端重新进入 Link a device 会生成新的会话；旧码不能继续完成 linking。

**确认模型**

- 【明确】单端确认在主手机：扫描/输入码 + 生物识别/PIN +（必要时）设备确认。
- 【推断】没有 SAS、没有两端数字比对、没有服务端管理员批准；新设备端没有“接受”按钮。

### 2.6 Signal Linked Devices

> 抓取说明：`support.signal.org` 与 Signal 博客在本环境直接抓取返回空内容；以下依据搜索引擎返回的 Signal 官方支持页/官方博客摘要。

**配对流程**

- 【明确】新设备（Signal Desktop/iPad）显示 QR；主手机 Settings → Linked devices → Link a new device，先用生物识别或设备解锁码验证（不是 Signal PIN），再扫描 QR；随后选择 Transfer Message History 或 Don't Transfer。([Signal Support: Linked Devices](https://support.signal.org/hc/en-us/articles/360007320551-Linked-Devices)，访问 2026-11-17，经搜索索引)
- 【明确】Signal 博客说明：新设备创建临时 provisioning address，并把地址与本地生成的 Curve25519 公钥放进 QR；主手机扫描后，主手机用该公钥加密 provisioning message 发给新设备。([Signal Blog: A Synchronized Start for Linked Devices](https://signal.org/blog/a-synchronized-start-for-linked-devices)，访问 2026-11-17，经搜索索引)
- 【明确】每个主手机最多 5 个 linked devices。([Signal Support: Troubleshooting multiple devices](https://support.signal.org/hc/en-us/articles/360007320451-Troubleshooting-multiple-devices)，访问 2026-11-17，经搜索索引)

**有效期与单次性**

- 【NOT FOUND】官方支持页没有写 QR 的有效期或自动刷新行为。
- 【NOT FOUND】官方支持页没有描述扫描后按设备名称二次确认的对话框；文档只写生物识别/解锁、扫描 QR、选择是否迁移历史。
- 【推断】QR 含临时 provisioning address 与新设备公钥；linking 完成后该会话不应复用，但官方未给“过期时间”数字。

**失败恢复**

- 【明确】官方故障页覆盖 “The QR code couldn't load”“Signal cannot link this device using your current network”“QR Code Detected – No Usable Data Found”，并建议检查网络、调整相机距离/亮度/窗口大小；没有“刷新按钮”描述。([Signal Support: Troubleshooting multiple devices](https://support.signal.org/hc/en-us/articles/360007320451-Troubleshooting-multiple-devices)，访问 2026-11-17，经搜索索引)

**确认模型**

- 【明确】主手机是授权端：生物识别/解锁 + 扫描 + 选择是否迁移历史。
- 【推断】单端确认；没有 SAS、没有两端比对、没有服务端管理员批准。

### 2.7 Telegram（QR login）

> 抓取说明：`core.telegram.org` 在本环境直接抓取返回空内容；以下依据搜索引擎返回的 Telegram 官方 API 文档摘要。

**配对流程**

- 【明确】想登录的客户端调用 `auth.exportLoginToken`，得到 `auth.loginToken`（含 token 与 expires）；token 做 base64url 编码后放入 `tg://login?token=...` URL，再显示为 QR。([Telegram API: Login via QR code](https://core.telegram.org/api/qr-login)，访问 2026-11-17，经搜索索引)
- 【明确】已登录的手机扫描并调用 `auth.acceptLoginToken` 接受；token 有效期 “usually 30 seconds”。([同上](https://core.telegram.org/api/qr-login)，访问 2026-11-17，经搜索索引)

**有效期、单次性与恢复**

- 【明确】token 过期后必须再次调用 `auth.exportLoginToken`，并 **自动生成新的 QR code**。([同上](https://core.telegram.org/api/qr-login)，访问 2026-11-17，经搜索索引)
- 【明确】错误码包括 `AUTH_TOKEN_INVALID`、`AUTH_TOKEN_INVALID1`、`AUTH_TOKEN_EXPIRED`、`AUTH_TOKEN_ALREADY_ACCEPTED`；`AUTH_TOKEN_EXPIRED` 要求重新扫描更新后的 QR。([同上](https://core.telegram.org/api/qr-login)，访问 2026-11-17，经搜索索引)
- 【推断】旧 QR 在过期/被接受后失效；界面应显示新 QR，而不是让用户继续扫旧码。

**确认模型**

- 【明确】手机端（已登录设备）扫描并接受；这是单端批准。
- 【推断，未直接核实】如果账号启用了 two-step verification，新客户端在扫描后通常还需要输入 cloud password；官方页面（经搜索索引）提到这条路径，但本次没有直接读到逐字说明，方法名也不在已核实范围内。([Telegram API: Login via QR code](https://core.telegram.org/api/qr-login)，访问 2026-11-17，经搜索索引)
- 【推断】没有 SAS、没有两端数字比对；手机端就是“批准设备”的一侧。

### 2.8 Tailscale

**配对/入网流程**

- 【明确】Auth key 可在生成时设置有效期 1–90 天；不指定则 90 天。key 可以是 one-off（只能连接一个设备）或 reusable（可连接多个设备）。([Tailscale Docs: Auth keys](https://tailscale.com/kb/1085/auth-keys)，访问 2026-11-17)
- 【明确】启用 device approval 后，新设备在获得批准前不能收发流量；设备端会显示 “awaiting approval” 通知。([Tailscale Docs: Device approval](https://tailscale.com/kb/1099/device-approval)，访问 2026-11-17)
- 【明确】Owner/Admin/IT admin 在 admin console 的 Machines 页面看到 “Needs approval” 标记，选择 Approve 后立即生效，无需重启。([同上](https://tailscale.com/kb/1099/device-approval)，访问 2026-11-17)
- 【明确】生成 auth key 时可勾选 Pre-approved，使用该 key 的设备自动通过 device approval。([同上](https://tailscale.com/kb/1099/device-approval)，访问 2026-11-17)

**有效期与单次性**

- 【明确】auth key 到期后，已由它授权的设备仍保持授权，直到 node key 到期；node key 默认 180 天。([Tailscale Docs: Auth keys](https://tailscale.com/kb/1085/auth-keys)，访问 2026-11-17)
- 【明确】one-off key 使用后会被 Tailscale 自动 revoke；revoke key 不会自动 deauthorize 已使用它的节点。([同上](https://tailscale.com/kb/1085/auth-keys)，访问 2026-11-17)
- 【NOT FOUND】device approval 本身没有写超时；文档只说共享设备只需批准一次。([Tailscale Docs: Device approval](https://tailscale.com/kb/1099/device-approval)，访问 2026-11-17)

**确认模型**

- 【明确】服务端/管理员批准：admin console 是批准端，设备端只显示 pending。
- 【明确】auth key 是“预授权”路径，可绕过人工批准。
- 【推断】没有 SAS、没有码比对；这是“服务端批准设备列表”的典型先例。

### 2.9 KDE Connect

**配对流程**

- 【明确】协议 `kdeconnect.pair`：一端发送 `pair: true` 请求；另一端回复 `pair: true` 接受或 `false` 拒绝。([KDE Connect protocol.md](https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md)，访问 2026-11-17)
- 【明确】配对请求包含 timestamp；“Used in the calculation of the pair verification code since protocol version 8”。([同上](https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md)，访问 2026-11-17)
- 【明确】协议约定请求 30 秒超时：“By convention the request times out after 30 seconds.”([同上](https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md)，访问 2026-11-17)
- 【明确】KDE UserBase 描述：任一端发起配对，另一端弹出接受提示；会出现 PIN，需要在电脑和手机上确认，桌面点 “Matches”、Android 点 “Pair”。([KDE UserBase: KDEConnect](https://userbase.kde.org/KDEConnect)，访问 2026-11-17；该页直接抓取返回 403，经搜索索引)

**有效期与单次性**

- 【明确】30 秒是协议约定，不是 wire format 强制字段。
- 【NOT FOUND】没有官方文档说明验证码本身的独立有效期；它绑定本次请求的 timestamp。

**失败恢复**

- 【推断】30 秒内未接受则请求超时；用户需要重新发起配对请求。没有自动刷新旧请求的机制。

**确认模型**

- 【明确】双端确认：一端请求，另一端接受；两端都看到验证码/ PIN 并确认匹配。
- 【推断】这是“无带外公钥绑定 + 人比对”的典型：码一致是安全机制，不是重复操作。

### 2.10 Sonos

**配对/添加流程**

- 【明确】添加 Sub：Sonos app → Settings → 选择房间 → Connect Sub → 按提示完成；第二个 Sub 用 Connect Second Sub。([Sonos Support: Set up your Sonos Sub](https://support.sonos.com/en/article/set-up-your-sonos-sub)，访问 2026-11-17)
- 【明确】添加环绕：Settings → System → 选择 home theater 房间 → Set up Surrounds；设备必须先在同一个房间、彼此可见。([Sonos Support: Add surround speakers to a Sonos home theater](https://support.sonos.com/en/article/add-surround-speakers-to-a-sonos-home-theater)，访问 2026-11-17)

**码、有效期与确认**

- 【明确】上述官方步骤没有出现配对码、PIN 或倒计时；流程由 app 提示和账号/局域网发现驱动。
- 【NOT FOUND】没有找到 Sonos 官方“配对码过期/刷新”的说明；这不适用于其当前流程。
- 【明确】失败时官方指向 “Product not found” 故障页；没有“刷新码”步骤。([Sonos Support: Set up your Sonos Sub](https://support.sonos.com/en/article/set-up-your-sonos-sub)，访问 2026-11-17)
- 【推断】单端/账号型：用户在 app 内选择设备并确认；没有两端码比对。

### 2.11 Philips Hue

**配对/添加流程**

- 【明确】Hue Bridge 的开发者 API 用 link button 做物理存在证明：未按按钮就 `POST /api` 会返回错误，要求按桥上的按钮；按下后再 POST 才创建 authorized user。官方解释是 “By pressing the button we prove that the user has physical access to the bridge.”([Philips Hue Developer: Get Started](https://developers.meethue.com/develop/get-started-2/)，访问 2026-11-17)
- 【明确】消费者 setup 页现在让用户在 Hue app 中扫描 Bridge 的 QR code；如果产品没有 QR code，可选择 “No QR code” 按搜索添加；页面也出现 “Hue Bridge found! Press the button in the center to connect”。([Philips Hue Support: How to set up Hue Bridge](https://www.philips-hue.com/en-us/support/connect-hue-product/accessories/hue-bridge)，访问 2026-11-17)

**有效期与单次性**

- 【明确】开发者页没有写 link button 的时间窗口。
- 【NOT FOUND】广泛流传的“按下按钮后 30 秒内必须完成请求”在本次抓取的 Hue 官方开发者页中没有出现；只有第三方集成文档这样写。因此本文不把 30 秒当作一手事实。
- 【推断】link button 是一次物理动作，按下后的一小段时间内允许创建 user；具体窗口以实现为准，官方未在抓取页说明。

**失败恢复**

- 【明确】如果没按按钮就请求，会收到错误；按按钮后重试同一 POST。([Philips Hue Developer: Get Started](https://developers.meethue.com/develop/get-started-2/)，访问 2026-11-17)
- 【推断】失败后用户重新按按钮即可，不需要“刷新码”。

**确认模型**

- 【明确】物理按钮 + app 请求：确认发生在桥一侧的物理动作，app 侧发起请求。
- 【推断】单端物理确认；没有两端数字比对、没有服务端操作员批准。

### 2.12 OAuth 2.0 Device Authorization Grant（RFC 8628）

**流程**

- 【明确】device authorization response 含 `device_code`、`user_code`、`verification_uri`、`expires_in`、`interval`；`expires_in` 是 device_code 与 user_code 的 lifetime。([RFC 8628 §3.2](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)
- 【明确】用户在浏览器打开 `verification_uri`，授权服务器要求输入 `user_code`；服务器 SHOULD 显示 user_code 并要求用户确认它和设备上显示的一致；客户端 MUST 仍显示 user_code，因为服务器会要求确认。([RFC 8628 §3.3/§3.3.1](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)
- 【明确】设备轮询 token endpoint；`authorization_pending` 表示用户还没完成；`slow_down` 要求 interval 增加 5 秒；`access_denied` 表示拒绝；`expired_token` 表示 device_code 过期、授权会话结束。([RFC 8628 §3.5](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)

**过期恢复**

- 【明确】除 `authorization_pending` / `slow_down` 外的错误，客户端 MUST stop polling and SHOULD react accordingly。([RFC 8628 §3.5](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)
- 【明确】过期后：“The client MAY commence a new device authorization request but SHOULD wait for user interaction before restarting.”([RFC 8628 §3.5](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)
- 【明确】客户端 SHOULD only commence a device authorization request when prompted by the user and not automatically。([RFC 8628 §3.1](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)

**确认模型**

- 【明确】单端批准：用户在浏览器/第二设备上登录并批准；设备端只轮询。设备端没有“接受”按钮，也没有两端 SAS。
- 【明确】`user_code` 的作用是让用户把“浏览器里的会话”和“设备上的请求”对应起来；§3.3.1 要求服务器显示 user_code 并让用户确认它匹配，客户端也必须继续显示 user_code，即使用户通过 `verification_uri_complete` 到达，仍要完成这个确认。

**对 UX 的直接启示**

- RFC 8628 是最接近“短时码 + 过期 + 重新发起”的规范先例，而且它明确反对静默自动重启；这与 Telegram 的自动刷新形成对照，说明“自动刷新”不是唯一正确做法，取决于码是否绑定到一次用户可感知的会话。

### 2.13 Windows Phone Link

**流程**

- 【明确】Android：在手机 Link to Windows 中选 Link your phone and PC，遇到 “Is the QR code on your PC ready?” 选 Continue，授权相机后扫描 PC 上的 QR。([Microsoft Support: Phone Link requirements and setup](https://support.microsoft.com/en-us/phone-link)，访问 2026-11-17，经搜索索引)
- 【明确】iOS：用系统相机扫描 QR 启动蓝牙配对；也可 “Pair manually”。([同上](https://support.microsoft.com/en-us/phone-link)，访问 2026-11-17，经搜索索引)
- 【明确】无障碍文档：Android “On your PC, type the code, ... select Continue”；iOS 在手机上点 Pair 后，PC 上也要选 Pair 并按 Enter。([Microsoft Support: Use a screen reader to navigate Phone Link app](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)，访问 2026-11-17)

**有效期**

- 【明确】“After three minutes, the QR code on your PC will expire.” 过期后 “restart the Phone Link app and try again.”([同上](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)，访问 2026-11-17)

**失败恢复**

- 【明确】“My codes don't match”：超过 3 次失败的 code matching 会阻止 linking；需要关闭并重新打开 Phone Link，重新开始 setup；之后会被要求接受权限而不是继续匹配码。([Microsoft Support: Troubleshooting setup for the Phone Link](https://support.microsoft.com/en-us/windows/apps/phonelink/troubleshooting-setup-for-the-phone-link)，访问 2026-11-17)
- 【明确】QR 过期后重启 app，而不是在原界面刷新旧码。([无障碍文档](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)，访问 2026-11-17)

**确认模型**

- 【明确】两端都有动作：手机扫码/输入码/点 Pair，PC 输入码或点 Pair；属于带码匹配的双端确认。
- 【推断】这里的 code matching 是配对码/蓝牙配对码，不是 SAS 派生的安全码；但它证明“两端都参与了同一次配对”。

### 2.14 Spotify Connect（补充）

**流程**

- 【明确】首次连接同一 WiFi 下的设备：打开 Spotify 播放，点 Connect 图标，选择设备；没有配对码/PIN。([Spotify Support: Spotify Connect](https://support.spotify.com/us/article/spotify-connect/)，访问 2026-11-17)
- 【明确】如果设备未登录账号，可先在设备上用 Spotify 凭据登录，之后即使不在同一 WiFi 也能在 app 中看到它。([同上](https://support.spotify.com/us/article/spotify-connect/)，访问 2026-11-17)
- 【明确】游戏主机是例外：PlayStation 上选 “LOG IN WITH PIN”，然后在另一设备访问 `www.spotify.com/pair` 输入屏幕上的 PIN。([Spotify Support: Spotify on PlayStation](https://support.spotify.com/us/article/spotify-on-playstation)，访问 2026-11-17)

**有效期与确认**

- 【NOT FOUND】官方页没有写主机 PIN 的有效期。
- 【推断】Spotify Connect 是账号/网络发现型，单端选择设备；主机 PIN 是账号登录授权，单端输入，没有两端码比对。

### 2.15 Syncthing（补充）

**流程**

- 【明确】两台设备要互相连接，必须在 **both devices** 上点 “Add Remote Device” 并输入对方的 Device ID；“Two devices will only connect and talk to each other if they are both configured with each other's device ID.”([Syncthing Docs: Getting Started](https://docs.syncthing.net/intro/getting-started.html)，访问 2026-11-17)
- 【明确】Device ID 是启动时密钥生成产生的 cryptographically-secure identifier；“configuration must be mutual”，所以 Device ID 不需要保密。([同上](https://docs.syncthing.net/intro/getting-started.html)，访问 2026-11-17)

**确认模型**

- 【明确】双端配置：双方都加入对方 ID；没有单端的“批准新设备”弹窗。
- 【明确】文档中的 “one time PIN + short device ID，双方验证对方知道 PIN 后才接受连接” 是一个 **proposal**，不是已文档化的当前功能。([Syncthing Docs: Understanding Device IDs](https://docs.syncthing.net/dev/device-ids.html)，访问 2026-11-17)
- 【推断】Syncthing 是“双方互加长期公钥身份”的模式，不是短时配对码模式。

### 2.16 Bluetooth Secure Simple Pairing：Numeric Comparison（补充）

**流程与确认**

- 【明确，经搜索索引】Numeric Comparison 适用于两端都能显示 6 位数字并能做 yes/no 输入的场景；两端各自显示 6 位数字，用户比对一致后在 **each device** 上确认。([NIST SP 800-121 Rev. 2: Guide to Bluetooth Security](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-121r2.pdf)，访问 2026-11-17；PDF 未能本地解析，以上据搜索索引摘要)
- 【明确，经搜索索引】Bluetooth Core Specification（Core 5.4，Vol 3 Part H Security Manager Specification）定义 Numeric Comparison：比较值取 `g2(PKax, PKbx, Na, Nb) mod 10^6` 的 6 位数字；两端显示，用户确认匹配；不匹配则中止协议并重新生成 nonce。([Bluetooth SIG: Core 5.4, Part H Security Manager Specification](https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Core-54/out/en/host/security-manager-specification.html)，访问 2026-11-17；该 HTML 抓取被截断，原文段落未直接读取，数值与步骤依据搜索索引及 NIST 文档)
- 【推断】这是最接近本项目 SAS 的先例：双端显示、双端人工确认；码一致是 MITM 防护的一部分，而不是“重复确认”。

## 3. 横向对比表

| 产品/规范 | 码类型 / 有效期 | 失效后恢复 | 是否自动刷新旧码 | 确认端数 | 人工比对 | 服务端批准 |
|---|---|---|---|---|---|---|
| Apple HomeKit / Home app | 8 位 setup code（静态，印在设备/包装）；Matter 11/21 位或 QR；官方未写过期 | 重置配件、检查网络/hub、重新添加；无“刷新码” | 否（静态码） | 单端（手机） | 无 | 无 |
| Apple AirPlay | 电视显示 onscreen password（4 位）；首次使用；官方未写过期 | 重新发起连接；安全策略在连接前生效 | 否（每次连接重新显示） | 单端（发送端输入） | 无（码在接收端显示） | 无 |
| Google Cast / Chromecast | 电视显示 QR 或 setup code；官方未写有效期 | 重启设备/路由器、恢复出厂、重新 setup | 否（文档导向重启） | 单端（手机） | 无 | 无 |
| Matter / Thread | QR 或 11/21 位 manual code；commissioning window 有时限（规范上限 15 分钟） | 窗口关闭后重新 open commissioning window；fail-safe 回滚 | 否（窗口关闭；OCW 生成新 passcode） | 单端（commissioner） | 无 | 无（commissioner 即管理员） |
| WhatsApp Linked Devices | QR 或 8 位码；官方未写有效期 | 主手机重新进入 Link a device；失败需重新授权 | NOT FOUND（二手称 QR 自动刷新） | 单端（主手机） | 无 | 无 |
| Signal Linked Devices | QR（临时 provisioning address + 公钥）；官方未写有效期 | 检查网络/相机后重新扫码；无刷新按钮文档 | NOT FOUND | 单端（主手机） | 无 | 无 |
| Telegram QR login | login token，约 30 秒 | 过期后自动生成新 QR；`AUTH_TOKEN_EXPIRED` 要求重扫新码 | 是（官方明确） | 单端（已登录手机接受） | 无 | 无（手机即批准端） |
| Tailscale | auth key（1–90 天，one-off/reusable）或交互登录 + device approval | key 过期需新 key；设备待批准时管理员 Approve；无批准超时文档 | 不适用 | 单端设备 + 管理员批准 | 无 | 有（admin console 的 Approve） |
| KDE Connect | 配对请求 + 验证码/PIN；请求约定 30 秒超时 | 超时后重新发起配对请求 | 否 | 双端 | 有（两端 PIN/code 匹配） | 无 |
| Sonos | 无码；app 选择设备 | app 故障排查/重新添加 | 不适用 | 单端（app） | 无 | 账号/局域网发现 |
| Philips Hue | Bridge QR 扫描；API 用 link button 物理确认；官方未写 30 秒 | 重新按按钮/重新扫描 | 否 | 单端物理确认 | 无 | 无（物理存在即授权） |
| RFC 8628 device flow | `user_code` + `device_code`，`expires_in` 明确 | `expired_token` 后停止轮询；可新发起，但 SHOULD 等用户操作 | 否（规范建议等待用户） | 单端（浏览器批准） | 有（用户确认 user_code 匹配） | 授权服务器批准 |
| Windows Phone Link | PC 显示 QR（3 分钟过期）；code matching | QR 过期重启 app；>3 次 code 失败需重启 setup | 否（重启 app） | 双端（扫码/输入 + PC Pair） | 有（code matching） | 无 |
| Spotify Connect | 无码（主机 PIN 例外） | 重新选择设备/登录账号 | 不适用 | 单端 | 无 | 账号登录 |
| Syncthing | Device ID（长期公钥身份） | 双方都添加对方 ID；无短时码 | 不适用 | 双端配置 | 无（ID 本身是公钥指纹） | 无 |
| Bluetooth SSP Numeric Comparison | 6 位数字（g2 mod 10^6；经搜索索引） | 不匹配则中止并生成新 nonce | 否（重新配对） | 双端 | 有（两端显示同一数字） | 无 |

## 4. 模式归纳

### 模式 1：静态长期码 + 重试（HomeKit、Hue 物理按钮）

- 码不是会话级 nonce，而是设备身份/物理存在的长期凭据；失败后用户重置设备或重新触发物理动作。
- 优点：没有倒计时和“刷新”概念，用户不会被旧码困住。
- 代价：码可能被拍照/泄露；安全性依赖物理接触、SRP/PAKE、设备重置后换钥。
- 对本项目：不可直接照搬，因为本项目明确要求短时、单次 nonce。

### 模式 2：短时轮转 QR，原地自动重生成（Telegram）

- token 约 30 秒过期；客户端再次调用 export 接口，自动显示新 QR；旧 token 失效。
- 优点：用户不需要理解“刷新”，过期后马上有新码可扫。
- 代价：如果用户正在用旧码，会看到“码变了”；需要处理 `AUTH_TOKEN_EXPIRED` 并引导重扫。
- 对本项目：可作为“过期后自动换码”的先例，但要注意本项目 nonce 被首个请求消耗后，自动换码不能让旧请求“复活”。

### 模式 3：限时会话 + 显式刷新/重开（RFC 8628、Phone Link、Matter OCW）

- 码有明确 TTL；过期后客户端停止旧流程，用户显式发起新流程（RFC 8628 的 user interaction、Phone Link 重启 app、Matter 重新 open commissioning window）。
- 优点：用户知道“旧码已死”，不会反复扫；避免后台无限生成码。
- 代价：多一步操作；需要清晰的“已过期/点此刷新”状态。
- 对本项目：与“失败必须换新 QR”的规则最兼容，且不需要改协议。

### 模式 4：单端证明完成（HomeKit、Matter、AirPlay、Google Cast、WhatsApp、Signal、Telegram、Tailscale）

- 一端拥有/输入码或批准，另一端的密码学证明（SRP、PASE、公钥加密、账号会话）完成后即结束；没有两端 SAS。
- 适用条件：码本身绑定到设备身份，或接收端能验证发送端的密钥/账号。
- 对本项目：QR 模式已经用二维码里的服务端公钥做 pinning，属于这一类；SAS 是额外的人工检查，不是协议完成的必要条件。

### 模式 5：双端批准 + 人工比对（Bluetooth NC、KDE Connect、Phone Link、Syncthing）

- 两端都要动作；其中 Bluetooth NC、KDE Connect、Phone Link 还要求人比对同一个短码/验证码。
- 适用条件：没有带外公钥绑定，或需要证明“两端的人都在场且看到的是同一次会话”。
- 对本项目：SAS fallback 模式属于这一类；QR 模式不属于，因为公钥已经带外绑定。

### 模式 6：服务端设备批准列表（Tailscale）

- 设备先完成认证/上线，进入 “Needs approval”；管理员在控制台看设备详情后 Approve；预授权 key 可跳过。
- 适用条件：有管理员角色、设备清单和撤销需求。
- 对本项目：服务端 “Accept device” 与此最接近；它不是码比对，而是授权决策。

### 模式 7：账号/网络自动发现，无配对码（Sonos、Spotify Connect）

- 同一账号/同一局域网内设备直接出现在列表；用户选择即用。
- 适用条件：账号体系 + 网络边界即可接受；不适用于需要防误配对的场景。
- 对本项目：不适用（本项目的信任边界是设备身份，不是账号）。

## 5. 对本项目 v2 的可借鉴结论

### 5.1 先把规格条款分清：QR 模式与 SAS 模式的确认要求不是同一句话

- `docs/wire-protocol-v2/pairing.md` §5 明确：nonce 被第一个格式正确的 `PairingRequest` 消耗，**不论配对最终成败**；失败必须换新 QR。这意味着“失败后刷新”不是可选 UX，而是协议状态机的必然结果。
- §6.2（QR 模式）只写：Server 在验证 `binding_echo` + `client_proof` 后，并且 “and local approval” 后返回 `PairingResult(OK)`；**没有在 QR 模式的消息序列里点名手机端 “Confirm pairing”**。
- §7.3 是 **SAS fallback** 小节，明确写 “Both ends require an explicit user action: the Client 'Confirm pairing', the Server 'Accept device' / `as-cmd` confirmation.”
- 因此，需要维护者确认：§7.3 的“双端显式确认”是只约束 SAS 模式，还是被产品要求推广到 QR 模式。这个区分决定了下面 (A)/(B) 的边界。

### (A) 不改协议、不改已签核规格即可做的 UX 改进

1. **失败即失效，立即停止展示旧 QR 和倒计时。**
   - 服务端 GUI 在收到 nonce-invalid / nonce-expired / pairing-failure 后，把 QR 区域切换为“此二维码已失效”状态，隐藏旧码，停止倒计时。
   - 先例：Telegram 的 `AUTH_TOKEN_EXPIRED` 要求重扫更新后的 QR；Phone Link QR 三分钟过期后要求重启 setup；RFC 8628 要求 `expired_token` 后停止轮询。([Telegram](https://core.telegram.org/api/qr-login)；[Phone Link](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)；[RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html)，访问 2026-11-17)

2. **提供显眼的“刷新二维码”按钮，一键在原地生成新 nonce/新 QR。**
   - 不需要关闭配对对话框再重开；按钮调用现有的“生成 nonce”路径即可。§5 只要求 nonce 单次、10 分钟、最多 8 个 outstanding；生成第 9 个会淘汰最旧，界面刷新一次只多一个，不违反规则。
   - 先例：Matter 的 `open-commissioning-window` 重新开窗并生成新 passcode；RFC 8628 的“新 device authorization request”；Phone Link 的“重启 setup”。([Matter chip-tool](https://project-chip.github.io/connectedhomeip-doc/development_controllers/chip-tool/chip_tool_guide.html)；[RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html)；[Phone Link](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)，访问 2026-11-17)

3. **默认不要“静默自动刷新”，除非产品明确选择。**
   - RFC 8628 明确 SHOULD wait for user interaction before restarting，且 SHOULD only commence when prompted by the user；自动刷新会让“旧码被扫”和“新码已生成”同时发生，用户难以判断自己扫的是哪一张。
   - 如果产品坚持要自动刷新，建议只在“倒计时自然到期且没有 in-flight 请求”时自动换码，并在 UI 上显示“二维码已更新，请重新扫描”；失败路径仍用显式按钮。
   - 反例先例：Telegram 是明确自动刷新，但它的 token 是纯登录 token，没有“失败后仍可能继续同一次配对”的语义；本项目 nonce 已消耗，自动换码解决不了旧请求。

4. **手机端错误要指向“让服务端刷新”，而不是“再扫一次”。**
   - 收到 nonce-invalid / nonce-expired 时，关闭扫描器或显示阻断态：“二维码已失效，请在服务端点击刷新后重新扫描”。不要保留旧码的扫描结果让用户重试。
   - 先例：Telegram 在 token 过期时要求重扫新码；Phone Link 在 QR 过期时要求重启 app。([Telegram](https://core.telegram.org/api/qr-login)；[Phone Link](https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app)，访问 2026-11-17)

5. **补上“等待另一端”的双向状态。**
   - 手机点 Confirm 后：显示“已确认，等待服务端接受”；服务端显示“手机已确认，等待操作员 Accept device”。
   - 服务端点 Accept 后：两端都进入成功态；如果服务端拒绝/超时，手机显示“服务端未接受，请刷新二维码重试”。
   - 先例：Tailscale 设备端显示 awaiting approval、管理员端看到 Needs approval；Bluetooth NC/KDE Connect 都是两端各自确认后才完成。([Tailscale](https://tailscale.com/kb/1099/device-approval)；[KDE protocol](https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md)，访问 2026-11-17)

6. **把 SAS 的语义写清楚，降低“重复”感。**
   - 文案区分“安全码（两端应一致）”与“确认按钮（授权动作）”：前者是人工比对，后者是同意授权；两者不是同一件事。
   - 在 QR 模式，二维码已经携带服务端公钥，SAS 不是 MITM 防护的必要条件，可以弱化为“可选核对”；在 SAS fallback 模式，SAS 才是主要防护，必须保留两端确认。
   - 先例：AirPlay/Google Cast 只让接收端显示、发送端输入，没有第二端 Accept；KDE Connect/Bluetooth NC 的双端确认是因为它们依赖人比对。([AirPlay](https://support.apple.com/en-us/102324)；[Google Cast](https://support.google.com/chromecast/answer/10109990?hl=en)；[Bluetooth NC](https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Core-54/out/en/host/security-manager-specification.html)，访问 2026-11-17)

7. **失败后的服务端状态要可恢复，不要要求 Cancel 重来。**
   - 在同一个配对对话框内提供“刷新二维码”；保留服务端操作员的上下文（设备名称、地址提示、错误原因），只更换 nonce/QR。
   - 这符合 §5 的“nonce 内存态、关闭对话框不消耗 nonce、重启作废”的设计；刷新只是生成新 nonce，不改变协议。

### (B) 需要维护者裁决的确认模型变更

1. **B1：§7.3 的“双端显式确认”是否适用于 QR 模式？**
   - 文本事实：§7.3 位于 “SAS fallback” 章节；§6.2 的 QR 消息序列只要求 Server 的 “local approval”，没有点名 Client 的 Confirm。
   - 若结论是“只约束 SAS”：QR 模式去掉手机端 Confirm 属于实现/UX 调整，不需要改已签核规格；若结论是“产品要求所有模式双端确认”，则去掉手机 Confirm 需要改规格/重新签核。
   - 建议维护者先做这个文本裁决，再决定 B2/B3。

2. **B2：QR 模式是否去掉手机端 “Confirm pairing”？**
   - 支持去掉的一手先例（单端完成）：HomeKit（手机输入 8 位码 + SRP）、Matter（commissioner 扫设备码 + PASE）、AirPlay（发送端输入电视上的码）、Google Cast（手机扫电视 QR）、WhatsApp/Signal/Telegram（手机端批准）、Tailscale（auth key 或管理员批准）。这些产品都没有要求“两端各点一次确认”。
   - 支持保留的一手先例（双端确认）：Bluetooth Numeric Comparison（原文经搜索索引）、KDE Connect、Windows Phone Link、Syncthing。它们的共同点是缺少 QR 这样的带外公钥绑定，或需要人比对来排除 MITM。
   - 本项目的 QR 模式已经在二维码里携带服务端完整公钥，手机在 TLS 握手阶段用 SPKI pin 验证；`client_proof` 又证明手机持有身份私钥。密码学上，手机端再点一次 Confirm 不是完成配对的必要条件；它主要提供“人明确同意”的仪式感。
   - 折中方案：保留 SAS 显示与短暂停留（例如至少 1–2 秒或要求用户勾选“已比对”），但不把“Confirm”作为独立协议步骤；服务端 Accept 仍是最终授权。这样既保留人工检查，又减少重复点击。

3. **B3：QR 模式是否去掉服务端 “Accept device”？**
   - 支持去掉的先例：HomeKit、Matter、AirPlay、Google Cast、WhatsApp、Signal、Telegram 都在一端证明完成后自动完成；没有服务端操作员二次批准。
   - 支持保留的先例：Tailscale 的 device approval 明确要求 Owner/Admin 在 admin console 批准；设备端只显示 pending。这说明“服务端批准设备列表”在需要管理员治理的场景是成立的。
   - 本项目的 QR 可能被现场无关人员扫到；服务端 Accept 是防“误扫/恶意抢先配对”的最后一道人审。建议保留；如果要去掉，应改为“服务端可配置自动接受 + 设备列表事后撤销”，并把这视为安全模型变更，而不是纯 UX。

4. **B4：SAS 在 QR 模式是否还需要强制显示/比对？**
   - QR 模式已有服务端公钥 pinning，SAS 对 MITM 的边际价值低；它的价值是让操作员确认“手机连的是我这台服务端”，以及满足“人对人”的可见性。
   - 若保留，建议标注为“核对码（两端应一致）”，不把它和“确认按钮”混为一谈；若不保留，应只在 SAS fallback 模式显示。
   - 这是产品/安全裁决，不是协议必然。

5. **B5：自动刷新策略。**
   - 若选择“倒计时结束自动换码”，需要明确：旧 nonce 是否立即作废（§5 已经是“首个请求即消耗”，未请求的 nonce 到期自然失效）；自动换码是否会在用户不知情时产生多个 outstanding nonce（上限 8）；失败路径是否也自动换码。
   - RFC 8628 的先例支持“显式刷新”，Telegram 的先例支持“自动轮转”；两者都可行，但必须选一个并写进产品行为，而不是让 GUI 停留在旧码。

6. **B6：失败后能否“重试同一 QR”。**
   - §5 已经规定不能；任何“Retry” UX 都必须生成新 nonce。若维护者希望“失败后保留同一 QR 再试一次”，那是协议/安全规则变更，会削弱“单次性”，不建议。
   - 可借鉴 Matter 的“窗口内重试”模型，但那要求 nonce 不被首个请求消耗；与本项目现行 §5 相反。保持现行规则更安全，UX 上用“刷新”而不是“重试”。

## 6. 来源清单

访问日期均为 2026-11-17。标注“经搜索索引”表示官方页面直接抓取被拒/返回空，内容取自搜索引擎返回的同一官方页面摘要。

### Apple

- Apple Support, “Add a smart home accessory to the Home app”: https://support.apple.com/en-us/104998
- Apple Support, “Set up accessories with Home on iPhone”: https://support.apple.com/en-gu/guide/iphone/iph125110541/ios
- Apple Support, “If you can't add a smart home accessory to the Home app”: https://support.apple.com/en-us/126198
- Apple Platform Security, “HomeKit communication security”: https://support.apple.com/guide/security/communication-security-sec3a881ccb1/web
- Apple Support, “If you're asked to enter an AirPlay password on Apple TV 4K and Apple TV HD”: https://support.apple.com/en-us/102324
- Apple Support, “Stream video and audio with AirPlay”: https://support.apple.com/en-tm/guide/mac-help/mchld7e543a0/mac

### Google

- Google Support, “Set up Chromecast with Google TV and Voice Remote”: https://support.google.com/chromecast/answer/10109990?hl=en
- Google Support, “Set up and manage guest mode”: https://support.google.com/chromecast/answer/6109286?hl=en
- Google Support, “Set up guest mode for Chromecast Audio”: https://support.google.com/chromecast/answer/6279388?hl=en
- Google Home Developers, “Commissioning”: https://developers.home.google.com/matter/primer/commissioning

### Matter / CSA

- Connectivity Standards Alliance, “Matter 1.0 Core Specification” (PDF; 直接抓取 403，规范正文经搜索索引): https://csa-iot.org/wp-content/uploads/2022/11/22-27349-001_Matter-1.0-Core-Specification.pdf
- Connectivity Standards Alliance, “Matter Specification Version 1.4.1” (PDF; 直接抓取 403): https://csa-iot.org/wp-content/uploads/2025/05/23-27349-007_matter-1-4-1-core-specification.pdf
- Build with Matter, “Commissioning”: https://handbook.buildwithmatter.com/how-it-works/commisioning
- Project CHIP, “Working with the CHIP Tool”: https://project-chip.github.io/connectedhomeip-doc/development_controllers/chip-tool/chip_tool_guide.html
- Amazon Alexa+ Developer Docs, “Best Practices to Commission Matter Devices”: https://developer.amazon.com/docs/alexaplus/smarthome/best-practices-commission-matter

### WhatsApp / Signal / Telegram

- WhatsApp Help Center, “How to link a device”: https://faq.whatsapp.com/1317564962315842 （经搜索索引）
- WhatsApp Help Center, “How to link a device with phone number”: https://faq.whatsapp.com/1324084875126592 （经搜索索引）
- Signal Support, “Linked Devices”: https://support.signal.org/hc/en-us/articles/360007320551-Linked-Devices （经搜索索引）
- Signal Support, “Troubleshooting multiple devices”: https://support.signal.org/hc/en-us/articles/360007320451-Troubleshooting-multiple-devices （经搜索索引）
- Signal Blog, “A Synchronized Start for Linked Devices”: https://signal.org/blog/a-synchronized-start-for-linked-devices （经搜索索引）
- Telegram API, “Login via QR code”: https://core.telegram.org/api/qr-login （经搜索索引）

### Tailscale

- Tailscale Docs, “Auth keys”: https://tailscale.com/kb/1085/auth-keys
- Tailscale Docs, “Device approval”: https://tailscale.com/kb/1099/device-approval

### KDE Connect / Syncthing / Bluetooth

- KDE Connect protocol, `protocol.md`: https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md
- KDE UserBase, “KDEConnect”: https://userbase.kde.org/KDEConnect （直接抓取 403，经搜索索引）
- Syncthing Docs, “Getting Started”: https://docs.syncthing.net/intro/getting-started.html
- Syncthing Docs, “Understanding Device IDs”: https://docs.syncthing.net/dev/device-ids.html
- Bluetooth SIG, “Core 5.4, Part H Security Manager Specification”: https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Core-54/out/en/host/security-manager-specification.html （HTML 抓取截断；Numeric Comparison 细节经搜索索引）
- NIST SP 800-121 Rev. 2, “Guide to Bluetooth Security”: https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-121r2.pdf （PDF 未能本地解析；Numeric Comparison 描述经搜索索引）

### Sonos / Spotify / Philips Hue

- Sonos Support, “Set up your Sonos Sub”: https://support.sonos.com/en/article/set-up-your-sonos-sub
- Sonos Support, “Add surround speakers to a Sonos home theater”: https://support.sonos.com/en/article/add-surround-speakers-to-a-sonos-home-theater
- Spotify Support, “Spotify Connect”: https://support.spotify.com/us/article/spotify-connect/
- Spotify Support, “Spotify on PlayStation”: https://support.spotify.com/us/article/spotify-on-playstation
- Philips Hue Developer, “Get Started”: https://developers.meethue.com/develop/get-started-2/
- Philips Hue Support, “How to set up Hue Bridge”: https://www.philips-hue.com/en-us/support/connect-hue-product/accessories/hue-bridge

### RFC / Microsoft

- RFC 8628, “OAuth 2.0 Device Authorization Grant”: https://www.rfc-editor.org/rfc/rfc8628.html
- Microsoft Support, “Phone Link requirements and setup”: https://support.microsoft.com/en-us/phone-link （经搜索索引）
- Microsoft Support, “Use a screen reader to navigate Phone Link app”: https://support.microsoft.com/en-us/accessibility/phone-link/use-a-screen-reader-to-navigate-phone-link-app
- Microsoft Support, “Troubleshooting setup for the Phone Link”: https://support.microsoft.com/en-us/windows/apps/phonelink/troubleshooting-setup-for-the-phone-link
