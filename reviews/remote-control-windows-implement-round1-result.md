---
type: implement-review-result
slug: remote-control-windows
round: 1
commit: 1152f3a
verdict: REVISE
blocking: 2
---

# implement remote-control-windows round1 — REVISE（blocking 2件）

対象: `2e98d76..1152f3a`（0cbf8a8 T001 / 2d4ec93 T003 / 205f3ca T002 / 1152f3a T004）
受入基準: `docs/specs/remote-control-windows/plan.md` AC-1〜AC-5
観点: correctness / simplicity / silent-failure / test-coverage / reuse-abstraction / security / design（design は UI 接触なしで対象外）

plan T001〜T003 の Interfaces と Steps は、参照行番号・API 名・feature 指定・cfg の網羅性まで含めて実装と一致している。windows-sys 0.59 の `OpenProcess` / `GetExitCodeProcess` / `TerminateProcess` / `WaitNamedPipeW` / `STILL_ACTIVE` / `ERROR_*` の存在と型は、ローカルの crate ソースで照合済み。差し戻すのは以下2点である。

---

## [required] Windows でパイプ名を他ローカルユーザーに先取りされると、fail-open のまま claude.ai OAuth トークンと全プロンプトが攻撃者へ渡る — `src/proxy/mod.rs:272-275` / `src/process/daemon.rs:183-206` / `src/process/launch.rs:174-180`

<details>
<summary>詳細</summary>

成立経路は4つのコード事実の連鎖であり、いずれも当該 SHA で確認した。

1. パイプ名 `\\.\pipe\claudex-<USERNAME>-proxy` は秘密ではなく、NPFS ルートは非管理者でも新規パイプ名を作成できる。攻撃者は permissive な DACL で先に同名パイプを作れる。
2. `spawn_pipe_listener` の `first_pipe_instance(true).create()` は既存インスタンスがあれば `ERROR_ACCESS_DENIED` で失敗するが、その分岐は `tracing::warn!` して `None` を返すだけで、proxy は TCP のみで正常起動を続ける（`src/proxy/mod.rs:272-275`）。
3. その warn はユーザーに届かない。`src/main.rs:54-58` により `run` サブコマンドでは stderr レイヤ自体が `None` にされ、ログはファイルにしか出ない。
4. `apply_remote_control_env` のガードは `is_proxy_running()`（自分が直前に書いた PID ファイルを見るので真）と `pipe_exists()` の AND だが、`pipe_exists()` は `WaitNamedPipeW` で名前の存在を見るだけで、そのパイプの作成者を一切検証しない（`GetNamedPipeServerProcessId` 相当の照合が無い）。攻撃者のパイプに対しても true を返す。

結果、`ANTHROPIC_UNIX_SOCKET` と `CLAUDE_CODE_OAUTH_TOKEN` が揃った状態で Claude Code が起動し、推論リクエスト一式が攻撃者のパイプへ流れる。攻撃者が得るのは claude.ai の access token、会話履歴と読み込んだファイルの全文、そして任意レスポンス（`tool_use` を含む）の注入経路である。

**反証の結果**:

- 「plan がスコープ外として承認済みでは」— 承認済みの範囲は *claudex が作るパイプの DACL*、すなわち「誰が我々のパイプに繋げるか」である。本件は逆向きの「我々が誰のパイプに繋ぐか」で、スコープ外宣言は及ばない。加えてその宣言の根拠「既存の 127.0.0.1 TCP 無認証と露出水準は同等で退行なし」は、この方向では成立しない。TCP は先取りされると `TcpListener::bind` が `?` で失敗し `start_proxy_background` が bail する fail-closed だが、パイプ側は fail-open である。**この非対称が本指摘の本体であり、承認済み判断の前提そのものが崩れている。**
- 「`first_pipe_instance(true)` が防いでいるのでは」— 防いでいるのは検出のみで、検出後の扱いが fail-open である。
- 「Windows 既定 DACL が守るのでは」— 守る対象は claudex 自身が作ったパイプであり、攻撃者が作ったパイプの DACL は攻撃者が決める。無関係。
- 「unix でも同じでは」— unix のソケットはユーザー所有の runtime ディレクトリ配下にあり、他ユーザーは先取りできない。本 diff が新規に開いた面である。

**修正の方向（1行）**: `first_pipe_instance(true)` の失敗を Remote Control にとって致命として扱い（proxy 起動を止めるか、launch 側が確実に bail する状態を残す）、あわせて `pipe_exists()` を「PID ファイルのプロセスがそのパイプのサーバであること」の照合に置き換える。

</details>

## [required] `stop_proxy` の Windows 経路が `TerminateProcess` の成否を見ずに「Terminated proxy」と表示し、PID ファイルを削除する — `src/process/daemon.rs:154-166`

<details>
<summary>詳細</summary>

correctness 観点と silent-failure 観点が独立に同一箇所を指した。

```rust
let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
if !handle.is_null() {
    unsafe { TerminateProcess(handle, 0); }   // 戻り値を見ていない
    unsafe { CloseHandle(handle); }
}
println!("Terminated proxy (PID {pid})");     // if の外。null でも必ず出る
```

直後の `remove_pid()?` は成否に関わらず実行される。

この経路は理論上の話ではなく、同じファイルのコードが自ら作り出している。`is_proxy_running()` は `OpenProcess` が null かつ `GetLastError() == ERROR_ACCESS_DENIED` のとき **生存** と判定する（`daemon.rs:100-107`）。まさにその状況では `OpenProcess(PROCESS_TERMINATE, ...)` も拒否されるので、「生きていると判定 → 殺せない → 殺したと表示 → PID ファイル削除」が一本の道として通る。unix 側は `kill(pid, 0)` が EPERM で false を返すため SIGTERM 経路に入らず、この誤表示は起きない。Windows 固有の新規経路である。

利用者は proxy が止まったと信じるが、実際にはプロセスが TCP ポートとパイプ名を掴んだまま残る。次の `claudex proxy start` は bind 失敗、`first_pipe_instance(true)` も失敗し、原因が見えない形で連鎖する。AC-5(3) の「stop 後にプロセスが実際に消えている」は実機検証者が確かめる項目だが、表示が常に成功なので検証者は誤った合格判定を出しうる。

**修正の方向（1行）**: `handle.is_null()` と `TerminateProcess` の戻り値を確認し、失敗時は成功文言を出さず PID ファイルも残す（あるいはエラーとして伝播する）。

</details>

---

## 指摘（非blocking）

- **[suggestion] README と `config.example.toml` の「Unix only」が4箇所残っている** — `README.md:45`, `README.md:217`, `config.example.toml:42`, `config.example.toml:388`。本 diff がこの記述を虚偽にした。T004 の未実施分（issue 投稿）と同じタイミングで直すのが自然である。Windows 検証者に渡す RC の同梱ドキュメントが「Unix 限定」と書いている状態は、引き継ぎの目的と正面から衝突する。
- **[suggestion] `is_proxy_running` が `GetExitCodeProcess` の失敗を「停止」と同一視する** — `src/process/daemon.rs:117-119`。API 呼び出し自体の失敗と「プロセスが死んでいる」を区別していない。呼び出し元の `stop_proxy` / `proxy_status` はこれを stale と見なして PID ファイルを黙って削除する。
- **[suggestion] `NamedPipeListener::accept` の無限リトライが恒久障害を warn ログだけで隠す** — `src/proxy/mod.rs:209-228`。500ms 間隔で無限に retry するため、権限剥奪等の回復しない失敗でも `axum::serve` タスクは生きているように見え、`proxy status` は running を返し続ける。試行回数の上限か、連続失敗時の `error!` への昇格がほしい。
- **[suggestion] `USERNAME` 未設定時のフォールバック名 `claudex-default-proxy` が全ユーザー共通の固定名になる** — `src/process/daemon.rs:54`。パイプ名前空間はマシン全体で共有されるため、この分岐に落ちた複数ユーザーは同じ名前を解決する。対話ログオンでは `USERNAME` は必ず設定されるので到達性は限定的だが、フォールバック先を衝突しない値にしておく価値はある。

## nit

- `WaitNamedPipeW(name, 0)` の `0` は `NMPWAIT_NOWAIT`（値 1）ではなく `NMPWAIT_USE_DEFAULT_WAIT`（値 0）である。コメントの「timeout 0」は即時復帰を意味しない（`daemon.rs:196`、windows-sys 0.59 の Pipes/mod.rs:28-30 で照合）。戻り値の真偽は変わらないため実害はない。
- Deviation Log に T004 の逸脱（version bump を fork/main ではなく feature ブランチで実施）が記載されていない。依頼本文には書かれているが、plan 側の記録に残っていない。
- CI の `windows-check` は `cargo check` のみで clippy を掛けていない（`ci.yml:44-47`）。Windows 専用コードの clippy warning は誰も見ない。

---

## 人間が最終判断すべき箇所

1. **required-1 を RC 前に直すか、制約付きで RC を先に出すか。** 攻撃には同一マシンの敵対的ローカルアカウントが要る。単独ユーザーの開発機では現実性が低い一方、企業の共有 Windows 端末では前提が揃う。RC の配布先がどちらかで判断が変わる。
2. **plan のスコープ外宣言を再確認する必要がある。** 「DACL 非設定でも既存 TCP と露出水準は同等で退行なし（ユーザー承認済み 2026-08-29）」という承認は、squatting 方向を評価していない。承認の射程を狭めるか、判断をやり直すかを決めるのは人間である。
3. **T004 が未完のまま AC-4 / AC-5 が未検証である。** PR マージ・タグ push・引き継ぎ issue の3つが残っており、plan の完了条件「引き継ぎ可能な状態」に到達していない。上記2件の修正をこの RC に含めるか、次の RC に回すかもここで決まる。
