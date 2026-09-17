---
type: implement-review
slug: remote-control-mitm-proxy
round: 1
commit: 4dc0b626164d8f6f923123d4af8e493d1fba8b46
targets:
  - Cargo.toml
  - Cargo.lock
  - README.md
  - config.example.toml
  - examples/fwd_poc.rs
  - examples/mitm_poc.rs
  - src/config/mod.rs
  - src/process/daemon.rs
  - src/process/launch.rs
  - src/proxy/forward/handoff.rs
  - src/proxy/forward/identity.rs
  - src/proxy/forward/integration_tests.rs
  - src/proxy/forward/mod.rs
  - src/proxy/forward/route.rs
  - src/proxy/forward/tls.rs
  - src/proxy/mod.rs
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md
---

## 参照文書

`docs/` は `.gitignore` により追跡対象外である。`git show <SHA>:docs/...` では開けないので、作業ツリーのファイルを直接読むこと。

| パス | 内容 |
|---|---|
| `docs/specs/remote-control-mitm-proxy/requirements.md` | 受入基準（FR-001〜FR-019） |
| `docs/specs/remote-control-mitm-proxy/design.md` | 基本設計。改訂系列 rev1・rev2 を frontmatter に記録 |
| `docs/specs/remote-control-mitm-proxy/plan.md` | 実装計画とタスク分解、Deviation Log、追加タスク T011 |
| `docs/specs/remote-control-mitm-proxy/verification.md` | 検証記録（静的ゲート、実機観察、観察できなかった項目） |

## 設計の改訂

本実装の途中で design を2回改訂している。いずれも frontmatter の `amendment` に記録がある。

| 系列 | 内容 | 承認 |
|---|---|---|
| rev1 | FR-013（上流接続でのプロキシ設定の扱い） | `reviews/remote-control-mitm-proxy-design-rev1-round1-result.md` で APPROVE |
| rev2 | 振り分け規則の非推論（Passthrough）前方一致リストを実機実測に差し替え | 2026-09-17 に人間承認。本レビューで確認対象 |

rev2 の改訂範囲は design の「## 振り分け規則」節と、実装側は `src/proxy/forward/route.rs` の `PASSTHROUGH_PREFIXES` および `test_classify_passthrough_prefixes`（コミット `4dc0b62`）。

## 未裁定の論点

次の2件は受入基準の文面と実装の前提がずれている。人間の裁定を保留したままレビューへ出している。詳細は `verification.md` の「未解決の項目」節にある。

| 論点 | 所在 |
|---|---|
| FR-010 の受入基準（proxy 停止状態で `claudex run` が起動を拒む）が `src/main.rs:74-80` の proxy 自動起動により観察できない。`src/main.rs` は本実装の変更対象外（`git diff b522620..HEAD -- src/main.rs` が空） | `verification.md` 未解決の項目 1 |
| FR-006 第1 の受入基準（`claudex proxy status` のメトリクスに計上）が、`proxy status` の出力とメトリクス端点の不在により観察できない。いずれも本実装の変更対象外 | `verification.md` 未解決の項目 2 |

## 実装のコミット範囲

`cd0301a`（PoC ベースライン）〜 `4dc0b62`（HEAD）。タスク単位でコミットしている。

```
4dc0b62 fix: T011 非推論の前方一致リストを実測に合わせる
e961439 test: T009 forward proxy の統合テストを足す
e3f2487 docs: T010 Remote Control の説明を forward proxy 方式へ
67ab313 feat: T008 旧 Unix ドメインソケット方式を削除する
b8dac6c feat: T006 forward listener を proxy に同居させる
316cc6a feat: T007 claudex run が HTTPS_PROXY 方式で起動する
9f9ff03 feat: T004 Proxy-Authorization から利用者を解決する
5f5c3fa feat: T005 パスを分類して行き先を振り分ける
666c49f feat: T003 forward モジュールの土台を作る
06e0a4c feat: T002 RemoteControlMode と forward_proxy_port を足す
3d6ff2f feat: T001 PoC の CONNECT で Proxy-Authorization を復号する
cd0301a chore: design 段階の forward proxy PoC を記録する
```

## 既知の事前障害

`cargo test --bin claudex` で `terminal::osc8::tests` の3件（`test_file_path_to_uri_relative`・`test_file_path_to_uri_with_line`・`test_absolute_path_existing_file`）が失敗する。本実装の着手前から失敗しており、`src/terminal/` は変更対象外である。

## 実行環境の注記

`claudex` は `[lib]` ターゲットを持たないバイナリクレートである。テストの実行は `cargo test --bin claudex <filter>` を使う（`cargo test --lib` は実行できない）。
