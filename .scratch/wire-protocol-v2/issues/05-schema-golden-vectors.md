# 05: Schemas and cross-language golden vectors

**What to build:** the language-neutral schema set and the golden test vectors that let Rust and Android implementations prove byte-compatibility against the specification, not against each other's encoders.

**Blocked by:** 03, 04

**Status:** resolved

**Type:** task

## Requirements

- `.proto` schema for control messages (pairing/auth, negotiation, subscription, configuration, errors, close), field numbers fixed and recorded; compiled by both the Rust Server and the Android Client from the same source of truth.
- Media header layout as a bit-exact table (every field, offset, width, byte order, flags) — no "library default" gaps.
- Golden vectors under `fixtures/wire-protocol-v2/` (language-neutral format, e.g. JSON + hex): control happy paths, each error-registry case, auth success/failure (including pairing-transcript binding), media header/payload samples including sequence wrap, `stream_instance_id` change, `config_generation` change, and at least one reduced-MTU payload alignment case.
- Vectors must cover the negative space: malformed lengths, unknown fields/messages, version/capability conflicts, state violations, revoked/invalid authorization.
- A verification harness expectation: any implementation (Rust or Kotlin) replays the same vector files and reports byte-level equality — no implementation-specific encoders in the loop.
- Record vector provenance: generated from the normative spec (03), not from the prototype's throwaway `BRIEF.md` format.

## Definition of done

Vectors reviewed against the normative spec table-by-table; maintainer acceptance resolves this ticket and completes the phase-2 gate ("Normative Specification Gate" in spec.md).

## Comments

### 2026-11-12 — drafted and machine-validated, awaiting maintainer review

`protos/wire-protocol-v2/control.proto` (package `audio.share.v2`) + `fixtures/wire-protocol-v2/`（48 案例 + manifest.json + test-keys.json + README 回放契约）on `ticket/05-schema-golden-vectors`，合入 `phase-2/wire-protocol-v2` @ `fdb1c48`（52 files, +4905）。文件已在主 checkout。

- **向量分布**：control-happy 6 · error-registry 25（每分带至少一个；102/205/206/503 未向量化并在 manifest 记录原因）· negative-space 5 · media 9（全字段、序列回绕、instance/generation 切换、reduced-MTU 对齐）· resource-limits 3（64 KiB 边界、128 B pre-auth、320 B PairingResponse）
- **机器验证已执行**：protoc 3.21.12 编译通过；105 个控制帧经 `--decode`/`--encode` 双向字节一致；3 个签名 openssl 独立验证（损坏变体确认失败）
- **固定测试钥**：test-keys.json 记录 TEST-ONLY 材料与推导公式（向量仅按验证回放，不按生成）
- **5 处规范缺口待裁决**（fixture 已把假设隔离成一行改动）：
  1. §4.1 分带表未定义 Negotiate/NegotiateResponse 的 oneof 编号 → 实现指派 **7/8**
  2. pairing.md §6.3 未给 `mode`/`protocol_major` 的 TLV 编码 → 用 ASCII `"qr"`/`"sas"` + u32 BE，binding_context 全量记录在 fixture
  3. §7.5 vs §7.1/§12 的 reserved 媒体 flag 位冲突 → 向量遵循已裁决的 ignore 规则
  4. §5.2 `PairingResult(201, detail "declined")` 违反 §4.2/TM-6 的 no-detail-pre-auth → declined 案例未向量化
  5. §5.2 CONNECTING 行的 "Error(401 or 402)" 不可判定 → 向量锚定 NEGOTIATING/READY 行

### 2026-11-12 — maintainer adjudication on the 5 gaps + one vector bug; final pass

1. **Negotiate/NegotiateResponse = 7/8 维持**：连接分带中下一组空闲编号，不冲突不重排；§4.1 正式登记，golden bytes 不变。
2. **TLV 编码维持 ASCII mode + u32 BE major**：mode = 小写 ASCII `qr`/`sas`（不是 enum 数字）；`protocol_major` 固定 4 字节大端（v2 = `00 00 00 02`），不是 varint 或单字节；同一规则覆盖 pairing 与 auth transcript；删除 pairing.md 中 "major 1 byte" 的冲突注释。
3. **Reserved media flags 维持接收忽略**：发送端必须清零 bit 2–31，接收端必须忽略 bit 2–31、不得判 malformed；修改 §7.5 并删除 "reserved flag bits set" 的 malformed 条件；保留现有 bit 2 向量，补 bit 31 向量。
4. **PairingResult(201)**：删除 detail "declined"（§4.2/TM-6 pre-auth no-detail 优先）；补审批拒绝向量：只返回 `PairingResult(201)` + `Close(201)`，断言无 detail、不持久化、响应 ≤128 B。
5. **CONNECTING 按错误性质拆分 401/402**：消息格式或通用校验失败 → 401 MALFORMED_MESSAGE；消息合法但状态不允许 → 402 INVALID_STATE；新增「CONNECTING 收到合法 Subscribe → 402」向量；PairingRequest 等专门规则仍优先。
6. **独立向量 bug**：unset-body 向量实际写成 length=0 空帧，未覆盖声明的 unknown-field 场景 → 修正为 `frame_hex = "000000027800"`。

落地原则：1–3 项不改现有字节；4–5 项补规则与向量；修正 unset-body；最后清除规范与 fixtures 中所有临时 assumption 标记。全部落地后本 ticket resolved，第 2 阶段（Normative Specification Gate）闭环。

### 2026-11-12 — final pass landed, ticket resolved; phase-2 gate closed

All six adjudication items executed on `ticket/05-adjudication-final` (11839bb spec/pairing amendments → 911ad2f fixtures +1/-fix → 16bcd87 marker purge), merged into `phase-2/wire-protocol-v2` @ `c564c1a`; the stale proto header notes cleaned separately @ `2f24a4b` (protoc parse OK).

- §4.1 registers Negotiate=7/NegotiateResponse=8; §5.2 CONNECTING split into decidable 401/402 rows; §7.5 reserved-flag rule rewritten (sender zero, receiver ignore, never malformed); PairingResult(201) no-detail + declined vector added; TLV encodings normative in pairing.md §6.3 ("major 1 byte" comment deleted); unset-body vector fixed to `000000027800`.
- Vectors now **51 cases** (incl. `connecting-valid-subscribe-402`, `error-201-pairing-declined` asserting no-detail/no-persist/≤128 B, `flags-reserved-bit31-ignored`); 113 frame steps pass the protoc 3.21.12 round-trip validator with 0 failures.
- 17 temporary-marker locations purged across 8 files; manifest provenance pinned to the normative-spec commit; `derivation_assumptions` entries now declarative normative citations (TEST-ONLY warning retained permanently).

**Normative Specification Gate (spec.md) is complete**: transport ADR + normative spec + state tables + schema + media layout + error registry + capability rules + threat model + pairing/key-lifecycle + cross-language golden vectors, all accepted. Next per spec Tracer-Bullet: minimal authenticated v2 control channel between Server and Android (new tickets needed).

### 2026-11-13 — vector erratum: declined-201 `requires_repairing`

`control/error-201-pairing-declined.json` was generating `Status` with `code=201` only, omitting the §6.2 registry default `requires_repairing=true`; its proof-invalid sibling encoded the flag, confirming an omission rather than intent. §4.2 ("senders MUST equal the §6 registry defaults") leaves no exception, so the vector was corrected to `12 05 08 c9 01 18 01` (frame length 19). Ticket 05's other vectors are unaffected.
