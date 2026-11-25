# 23: Build 阶段 1 — 桥接

**What to build:** 在不动 Kotlin 2.0.21 的前提下，把 Gradle/AGP/protobuf 升到桥接版本；生产与测试两套构建、签名、ABI、fixture 行为保持。

**Blocked by:** 14, 19, 20, 22。

**Status:** resolved

- [x] AGP 8.13.2、Gradle 8.13、protobuf 插件 0.10.1；Kotlin 保持 2.0.21。
- [x] `./gradlew test` 全绿；生产签名 release 构建成功；`-PtestSupport`/`-PdeviceTests` 测试构建成功。
- [x] generated protobuf 源码 diff 与 fixture replay 通过；`.proto` 未改。
- [x] 双 ABI `.so`、`packageRelease` / `merge*JniLibFolders` 任务仍存在。
- [x] `EMULATOR` 跑 ready/rebind/recreate/lock/stop 无回归。

## Comments

2026-11-17 — 实现完成并合入集成分支（merge `0a0d03f`，feature `78bf676`）。
Gradle 8.13（官方 SHA-256 校验）、AGP 8.13.2、protobuf 插件 0.10.1；Kotlin 2.0.21 与
proto runtime 4.28.2 未动。生成 protobuf 源码 27 文件零 diff；JVM 488 tests / 0 失败；
生产签名 APK 证书 `CN=Audio Share Beta`、双 ABI、badging 不变；测试构建 debug 签名且含 harness；
默认构建无 fixture 字符串；cargo test 全绿。
证据：`evidence/ticket-23/`。
**EMULATOR**（ready/rebind/recreate/lock/stop）尚未执行，记 NOT RUN；待 24/25 后在最终构建上统一采集，
届时将本票置 resolved。

## Comments

2026-11-18 — EMULATOR 补齐：在包含本阶段及后续阶段的最终集成 tip `5ff8936` 上，
16 KB AVD 的 ready/rebind/recreate/lock/stop 全部 PASS；真机 NOT RUN。
桥接阶段自身的构建门禁已在此前记录于 `evidence/ticket-23/`。本票 resolved。
