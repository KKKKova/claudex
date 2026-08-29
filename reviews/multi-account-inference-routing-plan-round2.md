---
type: plan-review
slug: multi-account-inference-routing
round: 2
commit: working-tree（docs/ が gitignore のため plan は非追跡。作業ブランチ feature/multi-account-inference-routing、HEAD は本依頼コミット）
targets:
  - docs/specs/multi-account-inference-routing/plan.md
acceptance: plan.md の「差分spec」節（中量経路。受入基準 AC-1〜AC-6）
---

前回指摘への対応一覧:

- required-1 → (a) を採用（ユーザー裁定 2026-08-29）。T003 Step 2 から `api_key_keyring` の併記を削除。スコープ外節の根拠を「書き込みだけ実装・読み出し無し。平文 `api_key` で運用、読み出し実装は別件」に書き換え
- required-2 → warn へ格下げ（ユーザー裁定 2026-08-29）。差分spec の「黙って無効になる」記述を「冗長なだけで機能は失われていない」へ訂正。AC-4 の Then を「警告を出して起動続行」に変更。T003 は純粋関数 `redundant_remote_control_warning` + `eprintln!` に変更し、テストも警告文の有無判定に変更
- required-3 → T001 Step 3 に4本目の分岐（system 先頭制約あり → direct.rs での system 注入が必要 → plan 更新して再承認）を追加。試行マトリクスの変則2を「system 無し（補助リクエスト形の代表）」として明示
- suggestion-1 → T002 Step 1 に「`custom_headers` に `anthropic-beta` があれば `apply_auth` は付けない」を追加、テスト項目にも追加
- suggestion-2 → T002 Step 1 に `sk-ant-` 始まりで `api`/`oat` いずれでもない値への warn を追加
- suggestion-3 → T002 Step 2 のログ片を `let auth = ...` の束縛形に修正
- suggestion-4 → T001 完了条件に「手順書と判定結果を fork issue #7 へ転記」を追加（ユーザー指示でもある）
- nit-1 → T001 Step 2 に「トークン本体は記録しない（先頭4文字+末尾4文字まで）」を追加
- nit-2 → T003 Step 2 の雛形コメントに「claude-max 雛形とは base_url も auth_type も別物」を追加
- 人間判断3（T001 両方不可時の落とし先）→ T001 Step 3 の該当分岐に「目標1の運用を継続する」を明記
