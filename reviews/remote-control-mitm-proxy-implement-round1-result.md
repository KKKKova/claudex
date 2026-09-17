---
type: implement-review-result
slug: remote-control-mitm-proxy
round: 1
commit: 4dc0b626164d8f6f923123d4af8e493d1fba8b46
verdict: REVISE
blocking: 2
status: in-review
reviewed_at: 2026-09-18
---

# implement remote-control-mitm-proxy round1 — REVISE（blocking 2件）

対象: `cd0301a..4dc0b62`（T001〜T011、forward proxy 方式一式）
受入基準: `docs/specs/remote-control-mitm-proxy/requirements.md` FR-001〜FR-019
観点: correctness / simplicity / silent-failure / test-coverage / reuse-abstraction / security / design（design は UI 接触なしで対象外）

指摘総数が7件を超えるため、まず required 2件のみ対応し、suggestion 以下は次round以降でよい。

---

## [required] proxy 停止時に CA 証明書の削除失敗を握り潰し、「消えた」と誤って断言する — `src/proxy/forward/handoff.rs:52-64` / `src/process/daemon.rs:145-148,158-174`

<details>
<summary>詳細</summary>

`cleanup()` は `remove_file` の `Err` を全て同じ枝で `tracing::debug!` に落とすだけで、呼び出し元へ成否を返さない。

```rust
pub fn cleanup() {
    if let Ok(path) = ca_pem_path() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::debug!(path = %path.display(), "cannot remove forward CA pem: {e}");
        }
    }
    ...
}
```

`stop_proxy()` はこの戻り値を見ずに `"the private CA for api.anthropic.com is gone with the proxy"` と無条件に表示する。`remove_file` が失敗する理由は「もともと存在しない」（想定内）だけでなく、権限拒否・アンチウイルスによる一時ロック・読み取り専用属性も含む。後者が起きた場合、`api.anthropic.com` を騙れる私設 CA の PEM が実行時ディレクトリに残ったまま、利用者は削除済みと信じる。

FR-012 の受入基準は「proxy 停止後、CA 証明書ファイルが 0 件残る」であり、この経路はその保証を実行時の環境条件（ファイルロックの有無）に依存させている。`proxy_status()` 側の cleanup 呼び出しも同じ構造で、テストも無い。

**反証**: 「もともと存在しないケースしか通らないのでは」という反論は成立しない。`ENOENT` と他の `io::ErrorKind`（`PermissionDenied` 等）は区別せずに同じ枝へ落ちており、コード上その区別を行っていないことを確認した。Windows のファイルロック（アンチウイルスのスキャン中ハンドル保持等）は実運用で起こり得る事象であり、机上の仮定ではない。

**修正の方向（1行）**: `cleanup()` の戻り値（削除できたか）を呼び出し元へ返し、失敗時は「消えた」と表示せず警告を出す。少なくとも `ENOENT` とそれ以外を区別する。

</details>

## [required] `relay_to` のヘッダ剥離（秘密情報が上流へ漏れない唯一の防御機構）に対する自動テストが皆無 — `src/proxy/forward/route.rs:270-285`

<details>
<summary>詳細</summary>

`relay_to` は上流へ送るヘッダから `host` / `proxy-authorization` / `proxy-connection` を落とし、`auth_profile` があるときはクライアントの `authorization` / `x-api-key` も落として `apply_auth` で付け直す。この処理こそが「合言葉やクライアントの資格情報が上流（本物の `api.anthropic.com` を含む）へ漏れない」という設計上の保証点だが、`integration_tests.rs` はいずれの経路でも wiremock が受信したリクエストの path しか検証しておらず、ヘッダの中身を確認していない。

security-adversary によるコードレビューでは、現時点の実装は正しくヘッダを剥離できていると確認済みである。しかし正しさの根拠がコード読解だけであり、自動テストが無い状態は、この関数への将来の変更（リファクタ・条件分岐の追加）が資格情報漏洩を静かに引き起こしても検知できないことを意味する。剥離ロジックの単純な条件反転や早期returnの1行変更で、`Proxy-Authorization` や `Authorization` がそのまま上流へ流れる状態を CI が見逃す。

**反証**: 「現状バグが無いなら指摘不要では」という反論は成り立たない。ここで問うているのは現時点の正しさではなく、退行を検知する手段の不在である。FR-006・FR-015 が明示的に要求する「上流へ資格情報が漏れないこと」を自動で保証する仕組みが、この diff の範囲に一つも無い。

**修正の方向（1行）**: `Proxy-Authorization` 付きで CONNECT した接続から Passthrough / DirectAnthropic 経由の両方でリクエストを送り、wiremock が受信したヘッダに `Proxy-Authorization` / （`auth_profile` あり時は）クライアントの `Authorization` が含まれないことを検証するテストを追加する。

</details>

---

## 指摘（非blocking）

<details>
<summary>suggestion 7件</summary>

- **[suggestion] `forward.json`（合言葉ファイル）が既定パーミッションで書かれ、特定の Linux 環境で同一ホストの別ユーザーから読める** — `src/proxy/forward/handoff.rs:29-40`。`XDG_RUNTIME_DIR` 未設定の Linux（`dirs::cache_dir()` へのフォールバック、umask 022 下）でのみ成立するため exploitability は低いが、成立すれば合言葉1本で config 内の全プロファイルの API キーが一度に露出する。同じリポジトリの `src/oauth/source.rs` は既に資格情報ファイルに 0600 を設定しており、`forward.json` だけ揃っていないのは意図的な判断というより取りこぼしに見える。
- **[suggestion] Proxy-Authorization ヘッダが壊れている場合（ヘッダ自体は存在するが base64/形式が不正）が「ヘッダなし」と区別なく無言で `Identity::Absent` に丸められる** — `src/proxy/forward/identity.rs:52-58`。合言葉不一致や該当プロファイル無しは `tracing::warn!` されるのに、この分岐だけログが無い。`Absent` は推論経路を素通しで本物の `api.anthropic.com` へ流すため、通信破損や将来のバグで発生した場合の手がかりが残らない。
- **[suggestion|criticality 7] `Identity::Unresolved`（合言葉は一致するが config にプロファイルが無い）分岐がテストに一度も現れない** — `src/proxy/forward/identity.rs:70-73`。`Unverified` は `test_inference_with_wrong_secret_returns_502` でカバー済みだが、`match` 内で隣り合う `Unresolved` は未証明。
- **[suggestion|criticality 7] `RouteClass::LegacyComplete`（`/v1/complete`）の `dispatch` 側実処理が統合テストに出てこない** — `src/proxy/forward/route.rs:178-197`。パス分類のユニットテストのみで、DirectAnthropic 解決時の中継、非DirectAnthropicでの404の両方が未検証。
- **[suggestion|criticality 6] 「profile は解決できたが DirectAnthropic ではない」分岐が再現されていない** — `src/proxy/forward/route.rs:67-79`。OpenAICompatible プロファイル（grok 等）で count_tokens を叩いた場合のローカル概算フォールバックが要件の Edge Cases に明記されているが自動テストが無い。
- **[suggestion|criticality 6] `apply_forward_proxy_env` の3つのガード（proxy 未起動／handoff ファイル欠落／CA pem 欠落）自体が未テスト** — `src/process/launch.rs`。テストされているのは副作用のない `forward_proxy_env` のみ。
- **[suggestion] 中継先ホストの検証が無く、`api.anthropic.com` 以外の宛先にもパス分類規則とクライアント資格情報が適用される** — `src/proxy/forward/route.rs:240-243,128`。合言葉を持つプロセスが絶対URI形式で任意ホスト宛の `/v1/messages` を送ると、`classify` が authority を見ずに `Inference` と分類してプロファイルのプロバイダへ中継する。design.md の契約は `api.anthropic.com` 宛を前提にしており、この経路では前提がコードで担保されていない。到達には CONNECT でなく絶対URI形式を送るクライアントが要るため exploitability は低い。

</details>

<details>
<summary>nit（最大3件）</summary>

- **[nit]** `is_some()` 直後に同じ `Option` を再取得して `unwrap()` している（`src/process/launch.rs:159-163`）。`if let Some(sig) = status.signal()` にすれば呼び出しが1回になり、CLAUDE.md の「production code で unwrap しない」規約にも合う。挙動は不変。
- **[nit]** `RouteClass::CountTokens` と `LegacyComplete` の `Some(profile)` 分岐が一字一句重複している（`src/proxy/forward/route.rs:157-198`）。`resolved_direct_anthropic_profile` 呼び出し後の共通処理を関数化できる。
- **[nit]** 自己ループ判定（FR-013）で `HTTPS_PROXY` の URL がパース不能な場合、ログなしで「自分自身を指していない」扱いになる（`src/proxy/forward/mod.rs:258-278`）。意図的な抑制であっても、パース不能時にどちらへ倒すかの理由をコメントに残す価値がある。

</details>

参考（confidence 80未満のため未採用、念のため共有）: `handle_conn` の CONNECT 判定が `peek` 8バイトに依存しており、TCP セグメント分割で先頭が7バイト未満だと非CONNECT経路へ誤送りする可能性がある（`src/proxy/forward/mod.rs:123-131`）。ループバック上での発生率は低いと見られ、確証は得られなかった。

## 人間が最終判断すべき箇所

1. **FR-010（proxy 停止状態で `claudex run` が起動を拒む）と FR-006 第1（`proxy status` のメトリクスに計上）の受入基準が、本 diff の変更対象外である `src/main.rs` の挙動と `proxy status` の出力形式に依存していて観察できない。** 依頼側が `verification.md` で保留と明記した論点であり、受入基準の文面を改めるか、別タスクとして残すかは人間が決める。
2. **`forward.json` の権限が 0600 になっていない点を、この round の required に含めるか、次round の suggestion のまま進めるか。** 既存の `oauth/source.rs` が同種のファイルに 0600 を適用している以上、対称性の欠如であり実装漏れの可能性が高い。ただし成立条件が `XDG_RUNTIME_DIR` 未設定の特定 Linux 環境に限られるため、required 昇格は見送った。この判断への異議は歓迎する。
3. **中継先ホストを検証しない設計を、この実装のスコープ内の欠陥として直すか、design.md 側の Non-Goal として明文化するかを決める必要がある。** 現状は「`api.anthropic.com` 宛にのみ振り分け規則を適用する」という設計の暗黙の前提が、コード上どこにも表現されていない。
