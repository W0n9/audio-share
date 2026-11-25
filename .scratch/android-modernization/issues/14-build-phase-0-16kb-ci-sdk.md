# 14: Build 阶段 0：16 KB 对齐与 CI SDK

**What to build:** 让 Android Client 的 native 库与 APK 满足 16 KB page size，并把 CI 的 SDK 组件升到目标版本；在 16 KB 模拟器上跑通现有设备用例。

**Blocked by:** 13。

**Status:** resolved

- [x] Rust 最终链接带 16 KB max/common page size 参数；arm64 与 x86_64 产物一致处理。
- [x] `llvm-readelf` 断言 `LOAD Align 2**14` 且 RELRO 末端 0x4000 对齐；`zipalign -c -P 16` 通过。
- [x] 16 KB 模拟器上 ready / rebind / recreate / lock / stop 五个用例通过，记录 `EMULATOR`。
- [x] CI 使用 build-tools 36.0.0 与 platforms android-37；NDK 保持 27.2。
- [x] 未改 `.proto`、ABI、cargo target、JNI 接口或签名配置。

## Comments

2026-11-17 — 实现完成并合入集成分支（merge commit `15fe72c`，feature commit `7f63225`）。
- 双 ABI `rustflags` 已加；`check-elf-alignment.sh` 对最终 `.so` 输出 `ALIGNED`
  （LOAD `0x4000`，RELRO 末端 `0x274000` / `0x2cc000`）；APK `zipalign -c -P 16` 通过。
- 生产/测试构建语义、`.proto`、ABI、JNI、版本号未变；JVM `./gradlew test -PnoNative=true` PASS。
- 证据：`evidence/ticket-14/`。
- **仍缺**：16 KB AVD `audio-share-api35-16k` 上的五个生命周期用例（ready/rebind/recreate/lock/stop）
  为 `NOT RUN`，由协调方在最终集成构建上串行执行；真机 `NOT RUN`。故本票在模拟器证据补齐前
  保持 `ready-for-agent`。

2026-11-18 — EMULATOR 补齐：最终集成 tip `5ff8936` 的测试签名 APK 在
`emulator:audio-share-api35-16k`（API 35 / x86_64 / PAGE_SIZE=16384）上
ready / rebind / recreate / lock / stop 全部 PASS（`evidence/ticket-08-refactor/emulator/`）。
真机 NOT RUN。本票 resolved。
