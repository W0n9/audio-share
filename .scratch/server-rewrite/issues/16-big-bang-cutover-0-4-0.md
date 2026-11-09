# 16: Big-bang cutover to 0.4.0

**What to build:** 删除旧 Server Core 与 MFC Server GUI；更新 README、协议说明、构建与发布文档；版本 `0.4.0`；CI 产出最终产物；形成干净的 release candidate。

**Blocked by:** 15 (v0.3.4 Android Client acceptance + soak)

**Status:** ready-for-agent

- [ ] 旧 Server Core 与 MFC Server GUI 从主分支移除。
- [ ] README 覆盖构建、运行、打包、Linux 依赖与已知限制。
- [ ] 版本号为 `0.4.0`，产物名与发布说明一致。
- [ ] CI 在 `windows-latest` 与 `ubuntu-24.04` 产出最终产物。
- [ ] 干净检出后可复现构建与测试。
- [ ] release candidate 通过 v0.3.4 Android Client 冒烟。
